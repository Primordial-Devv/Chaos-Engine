use std::collections::HashMap;

use chaos_core::{ChaosError, ChaosResult};

use crate::resources::{
    BufferHandle, MaterialBindingHandle, RenderTargetHandle, SamplerHandle, TextureFormat,
    TextureHandle, TextureKind,
};

/// Les statistiques d'une famille de ressources : compte vivant et octets
/// EXACTS (les octets uploadés à la création — l'estimation exploitable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindStats {
    /// Le nombre de ressources vivantes.
    pub alive: usize,
    /// Les octets uploadés cumulés des ressources vivantes.
    pub bytes: u64,
}

/// La photo des ressources GPU que possède le renderer : comptes, coûts,
/// retraites en attente — le moteur SAIT ce qu'il possède.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceStats {
    /// Les buffers (vertex, index) — comptes et octets.
    pub buffers: KindStats,
    /// Les textures — comptes et octets.
    pub textures: KindStats,
    /// Les samplers vivants.
    pub samplers: usize,
    /// Les pipelines créés (permanents en V1 — jamais détruits).
    pub pipelines: usize,
    /// Les meshes vivants.
    pub meshes: usize,
    /// Les materials vivants.
    pub materials: usize,
    /// Les bindings GPU vivants (un par material).
    pub bindings: usize,
    /// Les cibles hors écran — comptes et octets de PROFONDEUR (la
    /// couleur est comptée avec les textures).
    pub render_targets: KindStats,
    /// La shadow map interne du backend (0 ou 1 en V1) — comptes et
    /// octets (résolution² × 4, `Depth32Float`). Revient à zéro au
    /// `clear_directional_shadow`.
    pub shadow_maps: KindStats,
    /// Les ressources RETIRÉES du modèle, en attente de libération
    /// backend (vidées à la fin du prochain `render_frame`).
    pub retired: usize,
    /// Le coût total estimé : octets des buffers + textures + cibles +
    /// shadow maps.
    pub estimated_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct TextureInfo {
    pub(crate) label: String,
    pub(crate) bytes: u64,
    pub(crate) refs: usize,
    pub(crate) fallback: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: TextureFormat,
    pub(crate) kind: TextureKind,
    pub(crate) mip_levels: u32,
}

#[derive(Debug)]
pub(crate) struct SamplerInfo {
    pub(crate) label: String,
    pub(crate) refs: usize,
    pub(crate) fallback: bool,
}

#[derive(Debug)]
pub(crate) struct BufferInfo {
    pub(crate) label: String,
    pub(crate) bytes: u64,
    pub(crate) owner: Option<String>,
}

#[derive(Debug)]
pub(crate) struct RenderTargetInfo {
    pub(crate) label: String,
    pub(crate) depth_bytes: u64,
    pub(crate) color: TextureHandle,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: TextureFormat,
}

/// Une libération backend en attente — la ressource a déjà quitté le
/// modèle Chaos, le backend la relâchera au prochain point sûr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retired {
    Buffer(BufferHandle),
    Texture(TextureHandle),
    Sampler(SamplerHandle),
    Binding(MaterialBindingHandle),
    RenderTarget(RenderTargetHandle),
}

/// Le registre de durée de vie du renderer : identité, état, dépendances
/// et coût de chaque ressource — backend-agnostic. Les REFUS (ordre de
/// destruction incorrect, fallback protégé, handle périmé) se décident
/// ici, AVANT le backend ; les libérations backend passent par la
/// RETRAITE différée.
#[derive(Debug, Default)]
pub(crate) struct LifetimeTracker {
    textures: HashMap<TextureHandle, TextureInfo>,
    samplers: HashMap<SamplerHandle, SamplerInfo>,
    buffers: HashMap<BufferHandle, BufferInfo>,
    render_targets: HashMap<RenderTargetHandle, RenderTargetInfo>,
    pipelines: usize,
    retired: Vec<Retired>,
}

impl LifetimeTracker {
    pub(crate) fn register_pipeline(&mut self) {
        self.pipelines += 1;
    }

    /// Les textures FALLBACK builtin vivantes — les filets actifs des
    /// diagnostics.
    pub(crate) fn fallback_texture_count(&self) -> usize {
        self.textures.values().filter(|info| info.fallback).count()
    }

    /// Les samplers FALLBACK builtin vivants.
    pub(crate) fn fallback_sampler_count(&self) -> usize {
        self.samplers.values().filter(|info| info.fallback).count()
    }

    pub(crate) fn register_texture(&mut self, handle: TextureHandle, info: TextureInfo) {
        self.textures.insert(handle, info);
    }

    /// Les métadonnées d'une texture vivante — le registre de durée de
    /// vie est AUSSI le registre de métadonnées du renderer.
    pub(crate) fn texture_info(&self, handle: TextureHandle) -> Option<&TextureInfo> {
        self.textures.get(&handle)
    }

    pub(crate) fn register_sampler(&mut self, handle: SamplerHandle, label: &str, fallback: bool) {
        self.samplers.insert(
            handle,
            SamplerInfo {
                label: label.to_owned(),
                refs: 0,
                fallback,
            },
        );
    }

    pub(crate) fn register_buffer(
        &mut self,
        handle: BufferHandle,
        label: &str,
        bytes: u64,
        owner: Option<&str>,
    ) {
        self.buffers.insert(
            handle,
            BufferInfo {
                label: label.to_owned(),
                bytes,
                owner: owner.map(str::to_owned),
            },
        );
    }

    /// Prend une part sur CHAQUE texture listée et sur le sampler d'un
    /// material — le partage devient compté, slot par slot : un material
    /// qui met la même texture sur deux slots prend deux parts (la
    /// symétrie exacte du release).
    pub(crate) fn share_material_resources(
        &mut self,
        textures: &[TextureHandle],
        sampler: SamplerHandle,
    ) {
        for texture in textures {
            if let Some(info) = self.textures.get_mut(texture) {
                info.refs += 1;
            }
        }
        if let Some(info) = self.samplers.get_mut(&sampler) {
            info.refs += 1;
        }
    }

    /// Rend les parts prises par un material — slot par slot.
    pub(crate) fn release_material_resources(
        &mut self,
        textures: &[TextureHandle],
        sampler: SamplerHandle,
    ) {
        for texture in textures {
            if let Some(info) = self.textures.get_mut(texture) {
                info.refs = info.refs.saturating_sub(1);
            }
        }
        if let Some(info) = self.samplers.get_mut(&sampler) {
            info.refs = info.refs.saturating_sub(1);
        }
    }

    /// Retire une texture du modèle : refus si périmée, protégée
    /// (fallback builtin) ou encore partagée — l'ordre de destruction
    /// incorrect est une erreur explicite, jamais un effet silencieux.
    /// Les refs comptent des PARTS de slots (un material peut en tenir
    /// plusieurs sur la même texture), le message les nomme
    /// « material(s) » — la nuance est assumée.
    pub(crate) fn retire_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        let Some(info) = self.textures.get(&handle) else {
            return Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            )));
        };
        if info.fallback {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' is a builtin fallback and cannot be destroyed",
                info.label
            )));
        }
        if info.refs > 0 {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' is still used by {} material(s): destroy them first",
                info.label, info.refs
            )));
        }
        self.textures.remove(&handle);
        self.retired.push(Retired::Texture(handle));
        Ok(())
    }

    /// Retire un sampler du modèle — mêmes refus que les textures.
    pub(crate) fn retire_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        let Some(info) = self.samplers.get(&handle) else {
            return Err(ChaosError::Graphics(String::from(
                "sampler handle is stale or already destroyed",
            )));
        };
        if info.fallback {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}' is a builtin fallback and cannot be destroyed",
                info.label
            )));
        }
        if info.refs > 0 {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}' is still used by {} material(s): destroy them first",
                info.label, info.refs
            )));
        }
        self.samplers.remove(&handle);
        self.retired.push(Retired::Sampler(handle));
        Ok(())
    }

    /// Retire un buffer PUBLIC du modèle : refus si périmé ou possédé par
    /// un mesh (détruire le mesh, jamais ses organes).
    pub(crate) fn retire_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        let Some(info) = self.buffers.get(&handle) else {
            return Err(ChaosError::Graphics(String::from(
                "buffer handle is stale or already destroyed",
            )));
        };
        if let Some(owner) = &info.owner {
            return Err(ChaosError::Graphics(format!(
                "buffer '{}' is owned by mesh '{owner}': destroy the mesh instead",
                info.label
            )));
        }
        self.buffers.remove(&handle);
        self.retired.push(Retired::Buffer(handle));
        Ok(())
    }

    /// Retire un buffer POSSÉDÉ par un mesh que l'on détruit — le
    /// propriétaire emporte ses organes, sans passer par les refus.
    pub(crate) fn retire_owned_buffer(&mut self, handle: BufferHandle) {
        self.buffers.remove(&handle);
        self.retired.push(Retired::Buffer(handle));
    }

    /// Retire le binding d'un material détruit.
    pub(crate) fn retire_binding(&mut self, handle: MaterialBindingHandle) {
        self.retired.push(Retired::Binding(handle));
    }

    /// Vide la retraite — appelée par le renderer au point sûr (la fin du
    /// `render_frame` suivant). Rend la liste à libérer côté backend.
    pub(crate) fn drain_retired(&mut self) -> Vec<Retired> {
        std::mem::take(&mut self.retired)
    }

    pub(crate) fn retired_count(&self) -> usize {
        self.retired.len()
    }

    pub(crate) fn register_render_target(
        &mut self,
        handle: RenderTargetHandle,
        info: RenderTargetInfo,
    ) {
        self.render_targets.insert(handle, info);
    }

    /// Les métadonnées d'une cible vivante.
    pub(crate) fn render_target_info(
        &self,
        handle: RenderTargetHandle,
    ) -> Option<&RenderTargetInfo> {
        self.render_targets.get(&handle)
    }

    /// Retire une cible du modèle : refus si périmée ou si sa COULEUR est
    /// encore partagée par des materials (le refcount texture existant) ;
    /// la cible ET sa couleur partent en retraite.
    pub(crate) fn retire_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()> {
        let Some(info) = self.render_targets.get(&handle) else {
            return Err(ChaosError::Graphics(String::from(
                "render target handle is stale or already destroyed",
            )));
        };
        if let Some(color) = self.textures.get(&info.color)
            && color.refs > 0
        {
            return Err(ChaosError::Graphics(format!(
                "render target '{}': its color texture is still used by {} material(s): destroy them first",
                info.label, color.refs
            )));
        }
        let info = self
            .render_targets
            .remove(&handle)
            .ok_or_else(|| ChaosError::Graphics(String::from("render target vanished")))?;
        self.textures.remove(&info.color);
        self.retired.push(Retired::Texture(info.color));
        self.retired.push(Retired::RenderTarget(handle));
        Ok(())
    }

    pub(crate) fn render_target_stats(&self) -> KindStats {
        KindStats {
            alive: self.render_targets.len(),
            bytes: self
                .render_targets
                .values()
                .map(|info| info.depth_bytes)
                .sum(),
        }
    }

    pub(crate) fn texture_stats(&self) -> KindStats {
        KindStats {
            alive: self.textures.len(),
            bytes: self.textures.values().map(|info| info.bytes).sum(),
        }
    }

    pub(crate) fn buffer_stats(&self) -> KindStats {
        KindStats {
            alive: self.buffers.len(),
            bytes: self.buffers.values().map(|info| info.bytes).sum(),
        }
    }

    pub(crate) fn sampler_count(&self) -> usize {
        self.samplers.len()
    }

    pub(crate) fn pipeline_count(&self) -> usize {
        self.pipelines
    }
}
