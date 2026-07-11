//! Les RESSOURCES GPU possédées : buffers, textures (builtins, cache,
//! update), samplers (fallbacks), cibles hors écran — les limites
//! device appliquées avant le backend, la retraite différée, les stats
//! exactes.

use super::*;

impl Renderer {
    /// La borne DEVICE des textures (2D et faces de cube — WebGPU ne
    /// les distingue pas) : le refus nomme la valeur ET la limite —
    /// jamais une erreur de validation backend.
    fn check_texture_limit(&self, label: &str, width: u32, height: u32) -> ChaosResult<()> {
        let limit = self.capabilities.limits.max_texture_2d;
        if width > limit || height > limit {
            return Err(ChaosError::Graphics(format!(
                "texture '{label}': {width}x{height} exceeds the device texture limit ({limit})"
            )));
        }
        Ok(())
    }

    /// La borne DEVICE des buffers — le chemin COMMUN des buffers
    /// publics et des buffers de meshes.
    pub(super) fn check_buffer_limit(&self, label: &str, bytes: usize) -> ChaosResult<()> {
        let limit = self.capabilities.limits.max_buffer_bytes;
        if bytes as u64 > limit {
            return Err(ChaosError::Graphics(format!(
                "buffer '{label}': {bytes} bytes exceed the device buffer limit ({limit})"
            )));
        }
        Ok(())
    }

    /// Crée un buffer GPU (données uploadées à la création).
    pub fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle> {
        self.check_buffer_limit(&descriptor.label, descriptor.contents.len())?;
        let handle = self.backend.create_buffer(descriptor)?;
        self.lifetime.register_buffer(
            handle,
            &descriptor.label,
            descriptor.contents.len() as u64,
            None,
        );
        Ok(handle)
    }

    /// Détruit un buffer GPU. Refus explicites : handle périmé, ou buffer
    /// POSSÉDÉ par un mesh (détruire le mesh, jamais ses organes). La
    /// libération backend est différée au prochain point sûr (retraite).
    pub fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        self.lifetime.retire_buffer(handle)
    }

    /// Crée une texture GPU : applique la validation du descripteur —
    /// une erreur explicite avant tout appel GPU — puis délègue au backend.
    pub fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle> {
        self.create_texture_tracked(descriptor, false)
    }

    fn create_texture_tracked(
        &mut self,
        descriptor: &TextureDescriptor,
        fallback: bool,
    ) -> ChaosResult<TextureHandle> {
        descriptor.validate()?;
        self.check_texture_limit(&descriptor.label, descriptor.width, descriptor.height)?;
        let resolved = descriptor.resolved_mips();
        let handle = self.backend.create_texture(&resolved)?;
        self.lifetime.register_texture(
            handle,
            TextureInfo {
                label: resolved.label.clone(),
                bytes: resolved.expected_total_byte_len() as u64,
                refs: 0,
                fallback,
                width: resolved.width,
                height: resolved.height,
                format: resolved.format,
                kind: resolved.kind,
                mip_levels: resolved.mip_level_count(),
            },
        );
        Ok(handle)
    }

    /// Remplace les pixels du niveau 0 d'une texture — la mise à jour
    /// CONTRÔLÉE : handle vivant, jamais un fallback builtin, texture 2D
    /// mono-niveau seulement (limite V1 : recréer pour changer une
    /// texture mippée ou un cubemap), octets exacts. Validé AVANT le
    /// backend.
    pub fn update_texture(&mut self, handle: TextureHandle, pixels: &[u8]) -> ChaosResult<()> {
        let Some(info) = self.lifetime.texture_info(handle) else {
            return Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            )));
        };
        if info.fallback {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' is a builtin fallback and cannot be updated",
                info.label
            )));
        }
        if info.kind != TextureKind::D2 || info.mip_levels != 1 {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' cannot be updated in place: only single-level 2D textures support updates in V1 (recreate instead)",
                info.label
            )));
        }
        let expected =
            info.width as usize * info.height as usize * info.format.bytes_per_pixel() as usize;
        if pixels.len() != expected {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' expects {expected} bytes for an update, got {}",
                info.label,
                pixels.len()
            )));
        }
        self.backend.update_texture(handle, pixels)
    }

    /// Une texture builtin PROTÉGÉE, créée au premier usage puis
    /// partagée — les fallbacks adaptés aux usages (albédo, masques,
    /// normal maps du futur PBR).
    pub fn builtin_texture(&mut self, builtin: BuiltinTexture) -> ChaosResult<TextureHandle> {
        let descriptor = builtin.descriptor();
        if let Some(handle) = self.texture_cache.get(&descriptor.label) {
            return Ok(*handle);
        }
        let handle = self.create_texture_tracked(&descriptor, true)?;
        self.texture_cache.insert(descriptor.label.clone(), handle);
        Ok(handle)
    }

    /// Détruit une texture GPU. Refus explicites : handle périmé, fallback
    /// builtin protégé, environnement ACTIF (l'effacer d'abord), ou
    /// texture encore partagée par des materials (l'ordre de destruction
    /// incorrect est une erreur, jamais un effet silencieux). Toute
    /// entrée du cache pointant vers ce handle est évincée ; la
    /// libération backend est différée (retraite).
    pub fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        if self
            .environment
            .as_ref()
            .is_some_and(|environment| environment.cubemap == handle)
            && let Some(info) = self.lifetime.texture_info(handle)
        {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' is the active environment: clear it first",
                info.label
            )));
        }
        self.lifetime.retire_texture(handle)?;
        self.texture_cache.retain(|_, cached| *cached != handle);
        Ok(())
    }

    /// Cache de textures par clé logique — la clé est le `label` du
    /// descripteur (le futur chemin d'asset). Hit → handle existant ; miss →
    /// création (validation incluse) et insertion. Contrat V1 : la clé fait
    /// foi, pas le contenu — deux descripteurs différents sous le même label
    /// renvoient la première texture créée. `create_texture` reste le chemin
    /// brut qui crée toujours. Le partage par les materials est COMPTÉ par
    /// le registre de durée de vie ; l'éviction automatique sous pression
    /// mémoire viendra avec son besoin réel.
    pub fn get_or_create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> ChaosResult<TextureHandle> {
        if let Some(handle) = self.texture_cache.get(&descriptor.label) {
            debug!("texture cache hit '{}' ({handle:?})", descriptor.label);
            return Ok(*handle);
        }
        let handle = self.create_texture(descriptor)?;
        self.texture_cache.insert(descriptor.label.clone(), handle);
        Ok(handle)
    }

    /// Crée un sampler GPU — la manière de lire une texture, indépendante
    /// de la texture elle-même.
    pub fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle> {
        self.create_sampler_tracked(descriptor, false)
    }

    fn create_sampler_tracked(
        &mut self,
        descriptor: &SamplerDescriptor,
        fallback: bool,
    ) -> ChaosResult<SamplerHandle> {
        descriptor.validate()?;
        // La borne SOURCÉE du device (le 1..=16 du descripteur est son
        // self-check — la couche Renderer parle au nom du GPU).
        let ceiling = self.capabilities.limits.max_anisotropy;
        if descriptor.anisotropy > ceiling {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}': anisotropy x{} exceeds the device ceiling (x{ceiling})",
                descriptor.label, descriptor.anisotropy
            )));
        }
        let handle = self.backend.create_sampler(descriptor)?;
        self.lifetime
            .register_sampler(handle, &descriptor.label, fallback);
        Ok(handle)
    }

    /// Détruit un sampler GPU. Refus explicites : handle périmé, fallback
    /// builtin protégé, ou sampler encore partagé par des materials. La
    /// libération backend est différée (retraite).
    pub fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        self.lifetime.retire_sampler(handle)
    }

    /// Texture de repli des materials sans texture (`chaos.white`).
    pub(super) fn fallback_texture(&mut self) -> ChaosResult<TextureHandle> {
        self.builtin_texture(BuiltinTexture::White)
    }

    /// Sampler de repli builtin (`chaos.default_sampler`, Linear + Repeat)
    /// — PROTÉGÉ : le détruire est un refus explicite.
    pub(super) fn fallback_sampler(&mut self) -> ChaosResult<SamplerHandle> {
        if let Some(sampler) = self.fallback_sampler {
            return Ok(sampler);
        }
        let sampler =
            self.create_sampler_tracked(&SamplerDescriptor::new("chaos.default_sampler"), true)?;
        self.fallback_sampler = Some(sampler);
        Ok(sampler)
    }

    /// Crée une cible de rendu hors écran (couleur échantillonnable +
    /// profondeur propre), aux dimensions indépendantes de la fenêtre.
    pub fn create_render_target(
        &mut self,
        descriptor: &RenderTargetDescriptor,
    ) -> ChaosResult<RenderTargetHandle> {
        descriptor.validate()?;
        self.check_texture_limit(&descriptor.label, descriptor.width, descriptor.height)?;
        let (handle, color) = self.backend.create_render_target(descriptor)?;
        self.lifetime.register_texture(
            color,
            TextureInfo {
                label: descriptor.label.clone(),
                bytes: u64::from(descriptor.width)
                    * u64::from(descriptor.height)
                    * u64::from(descriptor.format.bytes_per_pixel()),
                refs: 0,
                fallback: false,
                width: descriptor.width,
                height: descriptor.height,
                format: descriptor.format,
                kind: TextureKind::D2,
                mip_levels: 1,
            },
        );
        self.lifetime.register_render_target(
            handle,
            RenderTargetInfo {
                label: descriptor.label.clone(),
                depth_bytes: u64::from(descriptor.width) * u64::from(descriptor.height) * 4,
                color,
                width: descriptor.width,
                height: descriptor.height,
                format: descriptor.format,
            },
        );
        debug!("render target '{}' created ({handle:?})", descriptor.label);
        Ok(handle)
    }

    /// La texture COULEUR d'une cible — l'entrée d'une passe ultérieure :
    /// elle se branche dans un material comme n'importe quelle texture.
    pub fn render_target_color(&self, handle: RenderTargetHandle) -> ChaosResult<TextureHandle> {
        self.lifetime
            .render_target_info(handle)
            .map(|info| info.color)
            .ok_or_else(|| {
                ChaosError::Graphics(String::from(
                    "render target handle is stale or already destroyed",
                ))
            })
    }

    /// Les dimensions d'une cible vivante.
    pub fn render_target_size(&self, handle: RenderTargetHandle) -> ChaosResult<(u32, u32)> {
        self.lifetime
            .render_target_info(handle)
            .map(|info| (info.width, info.height))
            .ok_or_else(|| {
                ChaosError::Graphics(String::from(
                    "render target handle is stale or already destroyed",
                ))
            })
    }

    /// Redimensionne une cible : l'ancienne part en retraite et un
    /// NOUVEAU handle est rendu — l'ancien handle ET son ancienne couleur
    /// deviennent périmés (le modèle générationnel fait foi) ; le
    /// consommateur re-résout la couleur et recrée son material.
    pub fn resize_render_target(
        &mut self,
        handle: RenderTargetHandle,
        width: u32,
        height: u32,
    ) -> ChaosResult<RenderTargetHandle> {
        let (label, format) = {
            let Some(info) = self.lifetime.render_target_info(handle) else {
                return Err(ChaosError::Graphics(String::from(
                    "render target handle is stale or already destroyed",
                )));
            };
            (info.label.clone(), info.format)
        };
        self.destroy_render_target(handle)?;
        self.create_render_target(&RenderTargetDescriptor::new(label, width, height, format))
    }

    /// Détruit une cible : refusé si sa couleur est encore partagée par
    /// des materials ; la cible et sa couleur partent en retraite
    /// (libération backend différée). Un handle périmé est une erreur
    /// explicite.
    pub fn destroy_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()> {
        let color = self
            .lifetime
            .render_target_info(handle)
            .map(|info| info.color);
        self.lifetime.retire_render_target(handle)?;
        if let Some(color) = color {
            self.texture_cache.retain(|_, cached| *cached != color);
        }
        Ok(())
    }

    /// Libère côté backend les ressources RETIRÉES du modèle — le point
    /// sûr : la frame vient d'être soumise, la précédente est passée.
    /// wgpu garantit déjà la survie des ressources en vol ; ce point fixe
    /// le CONTRAT de libération pour les futurs backends natifs.
    pub(super) fn flush_retired(&mut self) {
        for retired in self.lifetime.drain_retired() {
            let released = match retired {
                Retired::Buffer(handle) => self.backend.destroy_buffer(handle),
                Retired::Texture(handle) => self.backend.destroy_texture(handle),
                Retired::Sampler(handle) => self.backend.destroy_sampler(handle),
                Retired::Binding(handle) => self.backend.destroy_material_binding(handle),
                Retired::RenderTarget(handle) => self.backend.destroy_render_target(handle),
            };
            if let Err(release_error) = released {
                debug!("retired resource release failed: {release_error}");
            }
        }
    }

    /// La photo des ressources GPU possédées : comptes, coûts en octets
    /// (exacts — les octets uploadés), retraites en attente. Lecture
    /// froide — jamais sur le chemin chaud d'un draw.
    pub fn resource_stats(&self) -> ResourceStats {
        let buffers = self.lifetime.buffer_stats();
        let textures = self.lifetime.texture_stats();
        let render_targets = self.lifetime.render_target_stats();
        // La shadow map est un organe interne du backend : son coût est
        // dérivé des réglages (résolution² × 4 octets, Depth32Float) —
        // le backend n'a rien à compter.
        let shadow_maps =
            self.directional_shadow
                .as_ref()
                .map_or(KindStats::default(), |settings| KindStats {
                    alive: 1,
                    bytes: u64::from(settings.resolution) * u64::from(settings.resolution) * 4,
                });
        ResourceStats {
            buffers,
            textures,
            samplers: self.lifetime.sampler_count(),
            pipelines: self.lifetime.pipeline_count(),
            meshes: self.meshes.len(),
            materials: self.materials.len(),
            bindings: self.materials.len(),
            render_targets,
            shadow_maps,
            retired: self.lifetime.retired_count(),
            estimated_bytes: buffers.bytes
                + textures.bytes
                + render_targets.bytes
                + shadow_maps.bytes,
        }
    }
}
