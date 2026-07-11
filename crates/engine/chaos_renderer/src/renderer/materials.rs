//! Les MATERIALS : LE concept de surface — création validée par le
//! contrat du modèle, inspection, les mises à jour in-place (couleur,
//! PBR, cutoff, texture, sampler — jamais de recréation), destruction
//! qui rend ses parts.

use super::*;

impl Renderer {
    /// Crée un material — LA couche visuelle du moteur : un modèle
    /// (famille de shaders), des paramètres, des textures (fallbacks
    /// builtin : blanche 1×1, sampler Linear+Repeat), un état de rendu et
    /// une opacité. Le pipeline n'est plus l'affaire du consommateur : la
    /// permutation SURFACE est résolue immédiatement (un shader Custom
    /// invalide échoue ici, au bon endroit), les permutations de cibles à
    /// la première passe qui les demande. Les entrées material (texture,
    /// sampler, `base_color`) sont REFUSÉES si le modèle ne les consomme
    /// pas — jamais un effet silencieux.
    pub fn create_material(
        &mut self,
        descriptor: &MaterialDescriptor,
    ) -> ChaosResult<MaterialHandle> {
        Self::check_material_inputs(descriptor)?;
        let texture = match descriptor.texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let metallic_roughness_texture = match descriptor.metallic_roughness_texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let normal_map = match descriptor.normal_map {
            Some(texture) => texture,
            None => self.builtin_texture(BuiltinTexture::FlatNormal)?,
        };
        let occlusion_texture = match descriptor.occlusion_texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let emissive_texture = match descriptor.emissive_texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let textures = [
            texture,
            metallic_roughness_texture,
            normal_map,
            occlusion_texture,
            emissive_texture,
        ];
        for slot in textures {
            self.check_material_texture(&descriptor.label, slot)?;
        }
        let sampler = match descriptor.sampler {
            Some(sampler) => sampler,
            None => self.fallback_sampler()?,
        };
        {
            let mut context = PipelineContext {
                pipeline_cache: &mut self.pipeline_cache,
                sky_pipelines: &mut self.sky_pipelines,
                shadow_pipelines: &mut self.shadow_pipelines,
                instanced_pipelines: &mut self.instanced_pipelines,
                debug_pipelines: &mut self.debug_pipelines,
                backend: self.backend.as_mut(),
                shaders: &self.shaders,
                lifetime: &mut self.lifetime,
            };
            Self::resolve_material_pipeline(
                &mut context,
                &descriptor.model,
                descriptor.double_sided,
                descriptor.opacity,
                None,
            )?;
        }
        let binding = self
            .backend
            .create_material_binding(&MaterialBindingDescriptor {
                label: descriptor.label.clone(),
                texture,
                metallic_roughness_texture,
                normal_map,
                occlusion_texture,
                emissive_texture,
                sampler,
                params: MaterialParams {
                    base_color: descriptor.base_color,
                    metallic: descriptor.metallic,
                    roughness: descriptor.roughness,
                    receive_shadows: descriptor.receive_shadows,
                    alpha_cutoff: descriptor.alpha_cutoff,
                    emissive: descriptor.emissive,
                },
            })?;
        let record = MaterialRecord {
            label: descriptor.label.clone(),
            model: descriptor.model.clone(),
            base_color: descriptor.base_color,
            binding,
            texture,
            sampler,
            double_sided: descriptor.double_sided,
            opacity: descriptor.opacity,
            metallic: descriptor.metallic,
            roughness: descriptor.roughness,
            metallic_roughness_texture,
            normal_map,
            occlusion_texture,
            emissive: descriptor.emissive,
            emissive_texture,
            cast_shadows: descriptor.cast_shadows,
            receive_shadows: descriptor.receive_shadows,
            alpha_cutoff: descriptor.alpha_cutoff,
            frustum_culled: descriptor.frustum_culled,
        };
        let pool_handle = self
            .materials
            .insert(record)
            .ok_or_else(|| ChaosError::Graphics(String::from("material pool capacity exceeded")))?;
        self.lifetime.share_material_resources(&textures, sampler);
        let handle = MaterialHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!("material '{}' created ({handle:?})", descriptor.label);
        Ok(handle)
    }

    /// Les entrées material n'existent que si le modèle les consomme —
    /// une texture, un sampler, une couleur hors défaut sur un modèle
    /// sans entrées, ou une propriété PBR sur un modèle qui n'en lit pas
    /// (`Unlit`/`Lit`) : refusé en nommant la règle et la propriété,
    /// jamais inerte en silence.
    fn check_material_inputs(descriptor: &MaterialDescriptor) -> ChaosResult<()> {
        if !descriptor.model.material_inputs() {
            if descriptor.texture.is_some() || descriptor.sampler.is_some() {
                return Err(ChaosError::Graphics(format!(
                    "material '{}': its model has no material inputs — a texture or sampler would never be sampled",
                    descriptor.label
                )));
            }
            if descriptor.base_color != Color::WHITE {
                return Err(ChaosError::Graphics(format!(
                    "material '{}': its model has no material inputs — base_color would have no effect",
                    descriptor.label
                )));
            }
        }
        if !descriptor.model.pbr_inputs()
            && let Some(property) = descriptor.first_pbr_property()
        {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model does not consume PBR properties — '{property}' would have no effect",
                descriptor.label
            )));
        }
        if !descriptor.model.lighting_inputs() && !descriptor.receive_shadows {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model does not react to lighting — 'receive_shadows' would have no effect",
                descriptor.label
            )));
        }
        if descriptor.opacity == MaterialOpacity::Masked && !descriptor.model.material_inputs() {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model has no material inputs — Masked has no alpha to test \
                 against the cutoff",
                descriptor.label
            )));
        }
        if !descriptor.alpha_cutoff.is_finite()
            || descriptor.alpha_cutoff < 0.0
            || descriptor.alpha_cutoff > 1.0
        {
            return Err(ChaosError::Graphics(format!(
                "material '{}': alpha cutoff must be within 0..=1, got {}",
                descriptor.label, descriptor.alpha_cutoff
            )));
        }
        if descriptor.opacity != MaterialOpacity::Masked && descriptor.alpha_cutoff != 0.5 {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its opacity is not Masked — 'alpha_cutoff' would have no effect",
                descriptor.label
            )));
        }
        Ok(())
    }

    /// Un slot texture de material doit être 2D — les cubemaps
    /// attendront la passe environnement.
    fn check_material_texture(&self, label: &str, texture: TextureHandle) -> ChaosResult<()> {
        if let Some(info) = self.lifetime.texture_info(texture)
            && info.kind != TextureKind::D2
        {
            return Err(ChaosError::Graphics(format!(
                "material '{label}': texture '{}' is a cubemap — materials only sample 2D textures in V1 (the environment pass will consume cubemaps)",
                info.label
            )));
        }
        Ok(())
    }

    /// La photo d'un material vivant — l'inspection du futur éditeur :
    /// identité, modèle, paramètres courants, ressources résolues, état.
    pub fn material_info(&self, handle: MaterialHandle) -> ChaosResult<MaterialInfo> {
        let record = self
            .materials
            .get(PoolHandle {
                index: handle.index,
                generation: handle.generation,
            })
            .ok_or_else(|| {
                ChaosError::Graphics(String::from(
                    "material handle is stale or already destroyed",
                ))
            })?;
        Ok(MaterialInfo {
            label: record.label.clone(),
            model: record.model.clone(),
            base_color: record.base_color,
            texture: record.texture,
            sampler: record.sampler,
            double_sided: record.double_sided,
            opacity: record.opacity,
            metallic: record.metallic,
            roughness: record.roughness,
            metallic_roughness_texture: record.metallic_roughness_texture,
            normal_map: record.normal_map,
            occlusion_texture: record.occlusion_texture,
            emissive: record.emissive,
            emissive_texture: record.emissive_texture,
            cast_shadows: record.cast_shadows,
            receive_shadows: record.receive_shadows,
            alpha_cutoff: record.alpha_cutoff,
            frustum_culled: record.frustum_culled,
        })
    }

    /// Les paramètres uniformes courants d'un record — la monnaie des
    /// mises à jour in-place.
    fn params_from(record: &MaterialRecord) -> MaterialParams {
        MaterialParams {
            base_color: record.base_color,
            metallic: record.metallic,
            roughness: record.roughness,
            receive_shadows: record.receive_shadows,
            alpha_cutoff: record.alpha_cutoff,
            emissive: record.emissive,
        }
    }

    /// Le descripteur de binding reconstruit depuis un record — la
    /// monnaie des recréations transactionnelles (swap de texture ou de
    /// sampler).
    fn binding_descriptor(record: &MaterialRecord) -> MaterialBindingDescriptor {
        MaterialBindingDescriptor {
            label: record.label.clone(),
            texture: record.texture,
            metallic_roughness_texture: record.metallic_roughness_texture,
            normal_map: record.normal_map,
            occlusion_texture: record.occlusion_texture,
            emissive_texture: record.emissive_texture,
            sampler: record.sampler,
            params: Self::params_from(record),
        }
    }

    /// Met à jour la couleur de base d'un material vivant, EN PLACE : le
    /// buffer d'uniforms du binding est écrit, aucun binding ni pipeline
    /// n'est recréé — le chemin de modification par frame (tuning,
    /// animations de paramètres).
    pub fn set_material_color(
        &mut self,
        handle: MaterialHandle,
        base_color: Color,
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        if !record.model.material_inputs() {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model has no material inputs — base_color would have no effect",
                record.label
            )));
        }
        let binding = record.binding;
        let mut params = Self::params_from(record);
        params.base_color = base_color;
        self.backend.update_material_binding(binding, &params)?;
        if let Some(record) = self.materials.get_mut(pool_handle) {
            record.base_color = base_color;
        }
        Ok(())
    }

    /// Met à jour le facteur métallique d'un material PBR vivant, EN
    /// PLACE (aucune recréation).
    pub fn set_material_metallic(
        &mut self,
        handle: MaterialHandle,
        metallic: f32,
    ) -> ChaosResult<()> {
        self.update_pbr_params(handle, "metallic", |params| params.metallic = metallic)
    }

    /// Met à jour le facteur de rugosité d'un material PBR vivant, EN
    /// PLACE (aucune recréation).
    pub fn set_material_roughness(
        &mut self,
        handle: MaterialHandle,
        roughness: f32,
    ) -> ChaosResult<()> {
        self.update_pbr_params(handle, "roughness", |params| params.roughness = roughness)
    }

    /// Met à jour la couleur émissive d'un material PBR vivant, EN PLACE
    /// (aucune recréation) — le chemin des pulsations et du tuning.
    pub fn set_material_emissive(
        &mut self,
        handle: MaterialHandle,
        emissive: Color,
    ) -> ChaosResult<()> {
        self.update_pbr_params(handle, "emissive", |params| params.emissive = emissive)
    }

    /// Le chemin commun des paramètres PBR : refus si le modèle ne les
    /// consomme pas, écriture backend d'abord (le record ne bouge pas si
    /// le backend refuse), puis le record aligné.
    fn update_pbr_params(
        &mut self,
        handle: MaterialHandle,
        property: &'static str,
        apply: impl FnOnce(&mut MaterialParams),
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        if !record.model.pbr_inputs() {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model does not consume PBR properties — '{property}' would have no effect",
                record.label
            )));
        }
        let binding = record.binding;
        let mut params = Self::params_from(record);
        apply(&mut params);
        self.backend.update_material_binding(binding, &params)?;
        if let Some(record) = self.materials.get_mut(pool_handle) {
            record.base_color = params.base_color;
            record.metallic = params.metallic;
            record.roughness = params.roughness;
            record.emissive = params.emissive;
        }
        Ok(())
    }

    /// Met à jour le seuil d'élimination d'un material `Masked` vivant,
    /// EN PLACE (aucune recréation) — le chemin du tuning éditeur.
    /// Refus explicites : opacité non `Masked` (nommée), cutoff hors
    /// [0, 1] ou non fini, handle périmé.
    pub fn set_material_alpha_cutoff(
        &mut self,
        handle: MaterialHandle,
        alpha_cutoff: f32,
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        if record.opacity != MaterialOpacity::Masked {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its opacity is not Masked — 'alpha_cutoff' would have no effect",
                record.label
            )));
        }
        if !alpha_cutoff.is_finite() || !(0.0..=1.0).contains(&alpha_cutoff) {
            return Err(ChaosError::Graphics(format!(
                "material '{}': alpha cutoff must be within 0..=1, got {alpha_cutoff}",
                record.label
            )));
        }
        let binding = record.binding;
        let mut params = Self::params_from(record);
        params.alpha_cutoff = alpha_cutoff;
        self.backend.update_material_binding(binding, &params)?;
        if let Some(record) = self.materials.get_mut(pool_handle) {
            record.alpha_cutoff = alpha_cutoff;
        }
        Ok(())
    }

    /// Remplace la texture d'un material vivant (`None` → le fallback
    /// builtin) — TRANSACTIONNEL : validations puis nouveau binding, et
    /// seulement ensuite les parts déplacées et l'ancien binding retiré.
    /// Le handle du material SURVIT (même identité, nouvelle apparence) ;
    /// la même texture est un no-op.
    pub fn set_material_texture(
        &mut self,
        handle: MaterialHandle,
        texture: Option<TextureHandle>,
    ) -> ChaosResult<()> {
        let texture = match texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        if !record.model.material_inputs() {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model has no material inputs — a texture would never be sampled",
                record.label
            )));
        }
        if record.texture == texture {
            return Ok(());
        }
        let Some(info) = self.lifetime.texture_info(texture) else {
            return Err(ChaosError::Graphics(format!(
                "material '{}': the new texture is stale or already destroyed",
                record.label
            )));
        };
        if info.kind != TextureKind::D2 {
            return Err(ChaosError::Graphics(format!(
                "material '{}': texture '{}' is a cubemap — materials only sample 2D textures in V1 (the environment pass will consume cubemaps)",
                record.label, info.label
            )));
        }
        let (descriptor, sampler, old_texture, old_binding) = {
            let mut descriptor = Self::binding_descriptor(record);
            descriptor.texture = texture;
            (descriptor, record.sampler, record.texture, record.binding)
        };
        let binding = self.backend.create_material_binding(&descriptor)?;
        self.lifetime
            .release_material_resources(&[old_texture], sampler);
        self.lifetime.share_material_resources(&[texture], sampler);
        self.lifetime.retire_binding(old_binding);
        if let Some(record) = self.materials.get_mut(pool_handle) {
            record.texture = texture;
            record.binding = binding;
        }
        Ok(())
    }

    /// Remplace le sampler d'un material vivant (`None` → le fallback
    /// builtin) — même contrat transactionnel que la texture.
    pub fn set_material_sampler(
        &mut self,
        handle: MaterialHandle,
        sampler: Option<SamplerHandle>,
    ) -> ChaosResult<()> {
        let sampler = match sampler {
            Some(sampler) => sampler,
            None => self.fallback_sampler()?,
        };
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        if !record.model.material_inputs() {
            return Err(ChaosError::Graphics(format!(
                "material '{}': its model has no material inputs — a sampler would never be used",
                record.label
            )));
        }
        if record.sampler == sampler {
            return Ok(());
        }
        let (descriptor, texture, old_sampler, old_binding) = {
            let mut descriptor = Self::binding_descriptor(record);
            descriptor.sampler = sampler;
            (descriptor, record.texture, record.sampler, record.binding)
        };
        let binding = self.backend.create_material_binding(&descriptor)?;
        self.lifetime
            .release_material_resources(&[texture], old_sampler);
        self.lifetime.share_material_resources(&[texture], sampler);
        self.lifetime.retire_binding(old_binding);
        if let Some(record) = self.materials.get_mut(pool_handle) {
            record.sampler = sampler;
            record.binding = binding;
        }
        Ok(())
    }

    /// Détruit un material : ses parts sur la texture et le sampler sont
    /// rendues (ils redeviennent destructibles quand plus personne ne les
    /// partage), son binding part en retraite (libération backend
    /// différée). Un handle périmé est une erreur explicite.
    pub fn destroy_material(&mut self, handle: MaterialHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.remove(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        self.lifetime
            .release_material_resources(&record.textures(), record.sampler);
        self.lifetime.retire_binding(record.binding);
        debug!("material released ({handle:?})");
        Ok(())
    }
}
