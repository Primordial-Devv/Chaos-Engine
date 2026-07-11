//! La RÉSOLUTION de frame : de la file d'une passe au plan backend —
//! les contrats validés draw par draw, l'ordre à quatre temps (opaques
//! → masked → ciel → transparents triés), le FRUSTUM CULLING de la
//! passe, la MOISSON d'ombre (la visibilité de la LUMIÈRE, décorrélée),
//! et le BATCHING de l'instancing automatique (les runs (material,
//! mesh) fusionnés, l'ombre comprise). Le cœur chaud par-frame.

use super::*;

impl Renderer {
    /// Dérive la passe d'ombre du plan : des réglages posés ET une
    /// directionnelle qui projette — la PREMIÈRE de la collection
    /// (filtrée, tronquée : jamais un index hors du tableau GPU), la
    /// règle « les premières gagnent » de la troncature. Sans l'un ou
    /// l'autre → pas de passe : `enabled` reste à 0 côté GPU, le facteur
    /// d'ombre vaut 1 partout, rien de fatal. ZÉRO caster reste une
    /// passe : la map est effacée — jamais d'ombre fantôme d'une frame
    /// précédente.
    pub(super) fn derive_shadow_pass(
        &self,
        lights: &FrameLights,
        light_view: Option<Mat4>,
        casters: Option<(Vec<FrameDraw>, Vec<InstanceTransforms>)>,
    ) -> Option<FrameShadowPass> {
        let settings = self.directional_shadow.as_ref()?;
        let light_index = lights
            .lights
            .iter()
            .position(|light| matches!(light, Light::Directional { .. }))?;
        let (draws, instances) = casters.unwrap_or_default();
        Some(FrameShadowPass {
            // La vue de lumière précalculée avant la boucle des passes
            // (le même frustum a cullé la moisson) — recalculée en
            // secours si absente.
            view_projection: light_view.or_else(|| {
                let Light::Directional { direction, .. } = &lights.lights[light_index] else {
                    return None;
                };
                Some(light_view_projection(*direction, &settings.volume))
            })?,
            resolution: settings.resolution,
            depth_bias: settings.depth_bias,
            normal_bias: settings.normal_bias,
            light_index: u32::try_from(light_index).unwrap_or(0),
            draws,
            instances,
        })
    }

    /// La résolution commune des draws d'une passe : material → (pipeline
    /// PERMUTÉ pour le format de la destination, binding), mesh → buffers,
    /// transform → matrice. Les CONTRATS sont validés draw par draw, tout
    /// écart est écarté avec un warn — jamais fatal, jamais silencieux :
    /// material/mesh périmé, feedback (le material échantillonne la
    /// destination de sa passe), vertex layout du mesh désassorti du
    /// modèle, permutation irrésoluble (un warn par groupe de material).
    /// Les draws OPAQUES sortent avant les TRANSPARENTS (deux classes,
    /// le tri par material préservé dans chacune) — le tri fin par
    /// profondeur viendra avec la sous-phase transparence. Avec `sky`,
    /// le draw du CIEL s'insère entre les deux : après les opaques
    /// (fill-rate — LessEqual ne couvre que le fond) et avant les
    /// transparents (qui se mélangent par-dessus). Avec un collecteur
    /// `shadow_casters`, chaque draw OPAQUE d'un material `cast_shadows`
    /// y dépose sa copie d'ombre (pipeline depth-only de son layout,
    /// binding None) — la collecte vit ICI, à la branche opaque : le
    /// ciel injecté et les transparents ne peuvent jamais y fuir.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_pass_draws(
        materials: &ResourcePool<MaterialRecord>,
        meshes: &ResourcePool<MeshRecord>,
        context: &mut PipelineContext<'_>,
        commands: &[DrawCommand],
        color_format: Option<TextureFormat>,
        blocked_texture: Option<TextureHandle>,
        sky: bool,
        camera_position: Vec3,
        pass_frustum: &Frustum,
        light_frustum: Option<&Frustum>,
        mut shadow_casters: Option<&mut ShadowHarvest>,
    ) -> ResolvedPass {
        let mut opaque = Vec::with_capacity(commands.len());
        let mut opaque_keys = Vec::with_capacity(commands.len());
        let mut masked = Vec::new();
        let mut masked_keys = Vec::new();
        let mut transparent = Vec::new();
        let mut culled = 0;
        let mut groups: HashMap<BatchKey, BatchGroup> = HashMap::new();
        let mut memo: Option<(u32, VertexLayout, Option<PipelineHandle>)> = None;
        for command in commands {
            let material_handle = PoolHandle {
                index: command.material.index,
                generation: command.material.generation,
            };
            let Some(material) = materials.get(material_handle) else {
                warn!("draw dropped: stale material {:?}", command.material);
                continue;
            };
            if let Some(blocked) = blocked_texture
                && material.textures().contains(&blocked)
            {
                warn!(
                    "draw dropped: material '{}' samples the pass destination (feedback loop)",
                    material.label
                );
                continue;
            }
            // La file est triée par material : la permutation et le layout
            // attendu se résolvent UNE fois par groupe (mémoïsation), et un
            // échec de permutation ne warne qu'une fois par groupe.
            if memo
                .as_ref()
                .is_none_or(|(index, _, _)| *index != command.material.index)
            {
                let pipeline = Self::resolve_material_pipeline(
                    context,
                    &material.model,
                    material.double_sided,
                    material.opacity,
                    color_format,
                )
                .map_err(|resolve_error| {
                    warn!(
                        "draws dropped: material '{}' pipeline permutation failed: {resolve_error}",
                        material.label
                    );
                })
                .ok();
                memo = Some((
                    command.material.index,
                    material.model.expected_vertex_layout(),
                    pipeline,
                ));
            }
            let Some((_, expected_layout, pipeline)) = memo.as_ref() else {
                continue;
            };
            let Some(pipeline) = *pipeline else {
                continue;
            };
            let pool_handle = PoolHandle {
                index: command.mesh.index,
                generation: command.mesh.generation,
            };
            let Some(record) = meshes.get(pool_handle) else {
                warn!("draw dropped: stale mesh {:?}", command.mesh);
                continue;
            };
            if record.vertex_layout != *expected_layout {
                warn!(
                    "draw dropped: mesh {:?} vertex layout does not match the model of material '{}'",
                    command.mesh, material.label
                );
                continue;
            }
            let model = command.transform.matrix();
            // Les bounds MONDE, une fois par draw — partagés entre le
            // test du frustum de la LUMIÈRE (la moisson) et celui de LA
            // passe. `None` (mesh sans bounds) = jamais cullé.
            let world_bounds = record.bounds.map(|bounds| bounds.transformed(model));
            let draw = FrameDraw {
                pipeline,
                vertex_buffer: Some(record.vertex_buffer),
                index_buffer: record.index_buffer,
                element_count: record.element_count,
                model,
                normal: normal_matrix(model),
                binding: Some(material.binding),
                instances: None,
            };
            // L'identité de REGROUPEMENT du draw : le couple (material,
            // mesh) — la matière des runs que l'instancing fusionne.
            let key: BatchKey = (command.material.index, command.mesh.index);
            groups.entry(key).or_insert_with(|| BatchGroup {
                model: material.model.clone(),
                double_sided: material.double_sided,
                opacity: material.opacity,
            });
            // La participation aux ombres vient du CONTRAT de la
            // catégorie (Opaque et Masked projettent — masked en
            // silhouette pleine V1), jamais d'une règle locale. La
            // moisson a SA visibilité : le frustum de la LUMIÈRE — un
            // caster hors caméra projette encore une ombre visible,
            // jamais de « pop » d'ombre au bord de l'écran.
            if material.opacity.casts_shadows()
                && material.cast_shadows
                && let Some(harvest) = shadow_casters.as_deref_mut()
            {
                let lit_volume = !material.frustum_culled
                    || match (&world_bounds, light_frustum) {
                        (Some(bounds), Some(frustum)) => frustum.intersects(bounds),
                        _ => true,
                    };
                if !lit_volume {
                    harvest.culled += 1;
                } else if let Some(shadow_pipeline) = Self::resolve_shadow_pipeline(
                    context,
                    &record.vertex_layout,
                    material.double_sided,
                    false,
                ) {
                    harvest.draws.push(FrameDraw {
                        pipeline: shadow_pipeline,
                        binding: None,
                        ..draw
                    });
                    harvest.keys.push(key);
                    harvest
                        .groups
                        .entry(key)
                        .or_insert_with(|| (record.vertex_layout.clone(), material.double_sided));
                }
            }
            // La passe teste SON frustum — hors champ : compté, jamais
            // résolu plus loin (ni classe, ni tri, ni fusion).
            let visible = !material.frustum_culled
                || world_bounds
                    .as_ref()
                    .is_none_or(|bounds| pass_frustum.intersects(bounds));
            if !visible {
                culled += 1;
                continue;
            }
            match material.opacity {
                MaterialOpacity::Opaque => {
                    opaque.push(draw);
                    opaque_keys.push(key);
                }
                MaterialOpacity::Masked => {
                    masked.push(draw);
                    masked_keys.push(key);
                }
                MaterialOpacity::Transparent => transparent.push(draw),
            }
        }
        // L'ordre à quatre temps de la passe : opaques → masked (tous
        // deux écrivent la profondeur — les opaques d'abord, l'early-Z
        // aide les masked) → ciel → transparents TRIÉS. La ventilation
        // par catégorie compte les OBJETS logiques, AVANT que
        // l'instancing ne fusionne les runs.
        let mut breakdown = DrawBreakdown {
            opaque: opaque.len(),
            masked: masked.len(),
            transparent: transparent.len(),
            injected: 0,
        };
        // L'instancing automatique : les runs (material, mesh) des
        // classes qui écrivent la profondeur fusionnent — les
        // transparents restent des draws individuels (leur tri par
        // profondeur prime, V1 documentée).
        let mut instances = Vec::new();
        let mut opaque = Self::batch_class(
            context,
            color_format,
            opaque,
            &opaque_keys,
            &groups,
            &mut instances,
        );
        let mut masked = Self::batch_class(
            context,
            color_format,
            masked,
            &masked_keys,
            &groups,
            &mut instances,
        );
        opaque.append(&mut masked);
        // Le tri des transparents : ARRIÈRE → AVANT par distance² à la
        // caméra de SA passe (la translation du modèle comme proxy de
        // l'objet — le tri par triangle et l'OIT sont les extensions
        // notées), `total_cmp` (jamais un NaN qui panique), tri STABLE :
        // à distance égale, l'ordre de soumission gagne. Le regroupement
        // par material est SACRIFIÉ dans cette classe — la correction
        // avant le batching (les opaques gardent le leur).
        transparent.sort_by(|first, second| {
            let first_distance = (first.model.w_axis.truncate() - camera_position).length_squared();
            let second_distance =
                (second.model.w_axis.truncate() - camera_position).length_squared();
            second_distance.total_cmp(&first_distance)
        });
        if sky && let Some(pipeline) = Self::resolve_sky_pipeline(context, color_format) {
            opaque.push(FrameDraw {
                pipeline,
                vertex_buffer: None,
                index_buffer: None,
                element_count: 3,
                model: Mat4::IDENTITY,
                normal: Mat4::IDENTITY,
                binding: None,
                instances: None,
            });
            breakdown.injected = 1;
        }
        opaque.extend(transparent);
        ResolvedPass {
            draws: opaque,
            breakdown,
            instances,
            culled,
        }
    }

    /// Fusionne les RUNS consécutifs d'une classe (clé (material, mesh)
    /// égale, à partir de 2) en draws INSTANCIÉS : les transforms
    /// partent dans `instances`, le pipeline devient la permutation
    /// `vs_instanced` du groupe. Un groupe sans permutation (échec
    /// mémoïsé) reste en draws classiques — jamais la frame.
    fn batch_class(
        context: &mut PipelineContext<'_>,
        color_format: Option<TextureFormat>,
        class: Vec<FrameDraw>,
        keys: &[BatchKey],
        groups: &HashMap<BatchKey, BatchGroup>,
        instances: &mut Vec<InstanceTransforms>,
    ) -> Vec<FrameDraw> {
        let mut batched = Vec::with_capacity(class.len());
        let mut start = 0;
        while start < class.len() {
            let mut end = start + 1;
            while end < class.len() && keys[end] == keys[start] {
                end += 1;
            }
            let run = end - start;
            let pipeline = (run >= 2)
                .then(|| groups.get(&keys[start]))
                .flatten()
                .and_then(|group| Self::resolve_instanced_pipeline(context, group, color_format));
            if let Some(pipeline) = pipeline {
                let first = u32::try_from(instances.len()).unwrap_or(u32::MAX);
                for draw in &class[start..end] {
                    instances.push(InstanceTransforms {
                        model: draw.model,
                        normal: draw.normal,
                    });
                }
                let mut lead = class[start];
                lead.pipeline = pipeline;
                lead.instances = Some(InstanceRange {
                    first,
                    count: u32::try_from(run).unwrap_or(u32::MAX),
                });
                batched.push(lead);
            } else {
                batched.extend_from_slice(&class[start..end]);
            }
            start = end;
        }
        batched
    }

    /// Fusionne les casters d'ombre MOISSONNÉS sur toutes les passes :
    /// la moisson est d'abord TRIÉE par clé (les duplicatas
    /// multi-passes deviennent un seul run — l'ordre d'une passe de
    /// profondeur est indifférent), puis les runs ≥ 2 deviennent des
    /// draws instanciés sur la permutation d'ombre `vs_instanced` de
    /// leur (layout, culling).
    pub(super) fn batch_shadow_casters(
        context: &mut PipelineContext<'_>,
        harvest: ShadowHarvest,
    ) -> (Vec<FrameDraw>, Vec<InstanceTransforms>) {
        let ShadowHarvest {
            draws,
            keys,
            groups,
            culled: _,
        } = harvest;
        let mut keyed: Vec<(BatchKey, FrameDraw)> = keys.into_iter().zip(draws).collect();
        keyed.sort_by_key(|(key, _)| *key);
        let mut instances = Vec::new();
        let mut batched = Vec::with_capacity(keyed.len());
        let mut start = 0;
        while start < keyed.len() {
            let mut end = start + 1;
            while end < keyed.len() && keyed[end].0 == keyed[start].0 {
                end += 1;
            }
            let run = end - start;
            let pipeline = (run >= 2)
                .then(|| groups.get(&keyed[start].0))
                .flatten()
                .and_then(|(layout, double_sided)| {
                    Self::resolve_shadow_pipeline(context, layout, *double_sided, true)
                });
            if let Some(pipeline) = pipeline {
                let first = u32::try_from(instances.len()).unwrap_or(u32::MAX);
                for (_, draw) in &keyed[start..end] {
                    instances.push(InstanceTransforms {
                        model: draw.model,
                        normal: draw.normal,
                    });
                }
                let mut lead = keyed[start].1;
                lead.pipeline = pipeline;
                lead.instances = Some(InstanceRange {
                    first,
                    count: u32::try_from(run).unwrap_or(u32::MAX),
                });
                batched.push(lead);
            } else {
                batched.extend(keyed[start..end].iter().map(|(_, draw)| *draw));
            }
            start = end;
        }
        (batched, instances)
    }
}
