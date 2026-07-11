//! Les PIPELINES du renderer : le seul endroit qui en fabrique — la
//! création sur champs déstructurés et les CINQ caches de permutations
//! (materials, instanciées, ciel, debug, ombre), chacun mémoïsant ses
//! échecs avec un warn unique (le chemin dégradé, jamais la frame).

use super::*;

impl Renderer {
    /// Crée un pipeline graphique brut — TEST SEULEMENT depuis le Material
    /// System mature : les pipelines des materials sont des permutations
    /// résolues par le cache (`create_pipeline_with`), plus aucun chemin
    /// de draw public ne consomme un `PipelineHandle` brut.
    #[cfg(test)]
    pub(crate) fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> ChaosResult<PipelineHandle> {
        Self::create_pipeline_with(
            self.backend.as_mut(),
            &self.shaders,
            &mut self.lifetime,
            descriptor,
        )
    }

    /// La création de pipeline sur champs DÉSTRUCTURÉS — appelable depuis
    /// la boucle de résolution de frame (qui emprunte déjà les files).
    pub(super) fn create_pipeline_with(
        backend: &mut dyn GraphicsBackend,
        shaders: &ShaderLibrary,
        lifetime: &mut LifetimeTracker,
        descriptor: &PipelineDescriptor,
    ) -> ChaosResult<PipelineHandle> {
        let shader = match &descriptor.shader {
            ShaderRef::Named(name) => shaders.get(name).ok_or_else(|| {
                ChaosError::Graphics(format!("shader '{name}' not found in the library"))
            })?,
            ShaderRef::Inline(source) => source,
        };
        let handle = backend.create_pipeline(descriptor, shader)?;
        lifetime.register_pipeline();
        Ok(handle)
    }

    /// Résout la permutation de pipeline d'un material pour un format de
    /// destination — le cache déduplique : deux materials au même modèle
    /// et au même état partagent le même pipeline GPU.
    pub(super) fn resolve_material_pipeline(
        context: &mut PipelineContext<'_>,
        model: &MaterialModel,
        double_sided: bool,
        opacity: MaterialOpacity,
        color_format: Option<TextureFormat>,
    ) -> ChaosResult<PipelineHandle> {
        let key = PipelineKey {
            model: model.clone(),
            double_sided,
            opacity,
            color_format,
            instanced: false,
        };
        if let Some(handle) = context.pipeline_cache.get(&key) {
            return Ok(*handle);
        }
        // L'état de la permutation vient du CONTRAT de la catégorie
        // d'opacité (blend, entrée fragment, suffixe de label) — jamais
        // de règles locales.
        let mut label = format!("chaos.material.{}", model.tag());
        if double_sided {
            label.push_str(".double_sided");
        }
        label.push_str(opacity.label_suffix());
        if let Some(format) = color_format {
            label.push_str(&format!(".{format:?}"));
        }
        let mut descriptor = PipelineDescriptor::new(label, model.shader_ref())
            .with_vertex_layout(model.expected_vertex_layout())
            .with_cull_mode(if double_sided {
                CullMode::None
            } else {
                CullMode::Back
            })
            .with_fragment_entry(opacity.fragment_entry());
        if model.material_inputs() {
            descriptor = descriptor.with_material();
        }
        if opacity.blends() {
            descriptor = descriptor.with_transparency();
        }
        if let Some(format) = color_format {
            descriptor = descriptor.with_color_target(format);
        }
        let handle = Self::create_pipeline_with(
            context.backend,
            context.shaders,
            context.lifetime,
            &descriptor,
        )?;
        context.pipeline_cache.insert(key, handle);
        Ok(handle)
    }

    /// Résout la permutation INSTANCIÉE d'un material — le cache dédié
    /// (valeur `Option` : un échec de création — par exemple un shader
    /// `Custom` sans `vs_instanced` — est MÉMOÏSÉ avec un warn unique,
    /// et les runs de ce groupe restent des draws classiques, jamais la
    /// frame). Un `Custom` opte à l'instancing en exposant
    /// `vs_instanced` (la délégation documentée, le patron de
    /// `fs_masked`).
    pub(super) fn resolve_instanced_pipeline(
        context: &mut PipelineContext<'_>,
        group: &BatchGroup,
        color_format: Option<TextureFormat>,
    ) -> Option<PipelineHandle> {
        let key = PipelineKey {
            model: group.model.clone(),
            double_sided: group.double_sided,
            opacity: group.opacity,
            color_format,
            instanced: true,
        };
        if let Some(cached) = context.instanced_pipelines.get(&key) {
            return *cached;
        }
        let mut label = format!("chaos.material.{}", group.model.tag());
        if group.double_sided {
            label.push_str(".double_sided");
        }
        label.push_str(group.opacity.label_suffix());
        if let Some(format) = color_format {
            label.push_str(&format!(".{format:?}"));
        }
        label.push_str(".instanced");
        let mut descriptor = PipelineDescriptor::new(label, group.model.shader_ref())
            .with_vertex_layout(group.model.expected_vertex_layout())
            .with_instance_layout(instance_transforms_layout())
            .with_vertex_entry("vs_instanced")
            .with_cull_mode(if group.double_sided {
                CullMode::None
            } else {
                CullMode::Back
            })
            .with_fragment_entry(group.opacity.fragment_entry());
        if group.model.material_inputs() {
            descriptor = descriptor.with_material();
        }
        if group.opacity.blends() {
            descriptor = descriptor.with_transparency();
        }
        if let Some(format) = color_format {
            descriptor = descriptor.with_color_target(format);
        }
        let pipeline = Self::create_pipeline_with(
            context.backend,
            context.shaders,
            context.lifetime,
            &descriptor,
        )
        .map_err(|resolve_error| {
            warn!("instancing dropped: pipeline creation failed: {resolve_error}");
        })
        .ok();
        context.instanced_pipelines.insert(key, pipeline);
        pipeline
    }

    /// Résout la permutation du pipeline CIEL pour un format de
    /// destination — un cache dédié (le ciel n'est pas un material :
    /// triangle plein écran sans géométrie, LessEqual). Un échec de
    /// création est MÉMOÏSÉ avec un warn unique par format : le ciel est
    /// abandonné, jamais la frame.
    pub(super) fn resolve_sky_pipeline(
        context: &mut PipelineContext<'_>,
        color_format: Option<TextureFormat>,
    ) -> Option<PipelineHandle> {
        if let Some(cached) = context.sky_pipelines.get(&color_format) {
            return *cached;
        }
        let mut label = String::from("chaos.sky");
        if let Some(format) = color_format {
            label.push_str(&format!(".{format:?}"));
        }
        let mut descriptor = PipelineDescriptor::new(label, builtin::SKY)
            .with_depth_compare(DepthCompare::LessEqual);
        if let Some(format) = color_format {
            descriptor = descriptor.with_color_target(format);
        }
        let pipeline = Self::create_pipeline_with(
            context.backend,
            context.shaders,
            context.lifetime,
            &descriptor,
        )
        .map_err(|resolve_error| {
            warn!("sky dropped: pipeline creation failed: {resolve_error}");
        })
        .ok();
        context.sky_pipelines.insert(color_format, pipeline);
        pipeline
    }

    /// Résout la permutation du pipeline DEBUG pour un format de
    /// destination et un mode de profondeur — un cache dédié (le debug
    /// n'est pas un material : lignes monde, blend alpha, profondeur en
    /// LECTURE SEULE — testée en Scene, ignorée en Overlay). Un échec de
    /// création est MÉMOÏSÉ avec un warn unique par permutation : le
    /// debug est abandonné, jamais la frame.
    pub(super) fn resolve_debug_pipeline(
        context: &mut PipelineContext<'_>,
        color_format: Option<TextureFormat>,
        depth: DebugDepth,
    ) -> Option<PipelineHandle> {
        let key = (color_format, depth);
        if let Some(cached) = context.debug_pipelines.get(&key) {
            return *cached;
        }
        let mut label = String::from("chaos.debug");
        if depth == DebugDepth::Overlay {
            label.push_str(".overlay");
        }
        if let Some(format) = color_format {
            label.push_str(&format!(".{format:?}"));
        }
        let mut descriptor = PipelineDescriptor::new(label, builtin::DEBUG)
            .with_vertex_layout(DebugVertex::layout())
            .with_transparency()
            .with_depth_compare(match depth {
                DebugDepth::Scene => DepthCompare::LessEqual,
                DebugDepth::Overlay => DepthCompare::Always,
            });
        descriptor.topology = PrimitiveTopology::LineList;
        if let Some(format) = color_format {
            descriptor = descriptor.with_color_target(format);
        }
        let pipeline = Self::create_pipeline_with(
            context.backend,
            context.shaders,
            context.lifetime,
            &descriptor,
        )
        .map_err(|resolve_error| {
            warn!("debug dropped: pipeline creation failed: {resolve_error}");
        })
        .ok();
        context.debug_pipelines.insert(key, pipeline);
        pipeline
    }

    /// Résout la permutation du pipeline d'OMBRE pour un vertex layout et
    /// un état de culling — un cache dédié (l'ombre n'est pas un
    /// material : profondeur seule, vertex uniquement, groupe(0) réduit).
    /// Un layout SANS position (`Float32x3` à la location 0) ou un échec
    /// de création est MÉMOÏSÉ avec un warn unique par permutation : le
    /// caster est écarté, jamais la frame.
    pub(super) fn resolve_shadow_pipeline(
        context: &mut PipelineContext<'_>,
        vertex_layout: &VertexLayout,
        double_sided: bool,
        instanced: bool,
    ) -> Option<PipelineHandle> {
        let key = (vertex_layout.clone(), double_sided, instanced);
        if let Some(cached) = context.shadow_pipelines.get(&key) {
            return *cached;
        }
        let has_position = vertex_layout.attributes.iter().any(|attribute| {
            attribute.location == 0 && attribute.format == VertexAttributeFormat::Float32x3
        });
        if !has_position {
            warn!(
                "shadow casting dropped: the vertex layout carries no Float32x3 position at location 0"
            );
            context.shadow_pipelines.insert(key, None);
            return None;
        }
        let mut label = format!("chaos.shadow.{}", vertex_layout.stride);
        if double_sided {
            label.push_str(".double_sided");
        }
        if instanced {
            label.push_str(".instanced");
        }
        let mut descriptor = PipelineDescriptor::new(label, builtin::SHADOW)
            .with_vertex_layout(vertex_layout.clone())
            .with_cull_mode(if double_sided {
                CullMode::None
            } else {
                CullMode::Back
            })
            .with_depth_only();
        if instanced {
            descriptor = descriptor
                .with_instance_layout(instance_transforms_layout())
                .with_vertex_entry("vs_instanced");
        }
        let pipeline = Self::create_pipeline_with(
            context.backend,
            context.shaders,
            context.lifetime,
            &descriptor,
        )
        .map_err(|resolve_error| {
            warn!("shadow casting dropped: pipeline creation failed: {resolve_error}");
        })
        .ok();
        context.shadow_pipelines.insert(key, pipeline);
        pipeline
    }
}
