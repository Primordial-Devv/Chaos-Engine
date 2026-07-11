//! L'ORCHESTRATEUR du rendu. Ce fichier tient la DÉFINITION du
//! `Renderer` (le struct, les types transverses de la résolution) et la
//! COORDINATION de frame (`render_frame`, `render_to_target`) — chaque
//! famille de responsabilités vit dans son module enfant, qui accède
//! aux champs privés du parent (la visibilité des descendants — jamais
//! de getters artificiels). La carte :
//!
//! - `pipelines.rs` — la fabrique : les cinq caches de permutations
//! - `resolve.rs` — le cœur chaud par-frame : de la file au plan
//!   (opacité, culling, batching, moisson d'ombre)
//! - `resources.rs` — buffers, textures, samplers, cibles : posséder
//!   les ressources GPU (limites, retraite, stats)
//! - `materials.rs` — le concept de surface : création validée, mises
//!   à jour in-place, destruction
//! - `meshes.rs` — la géométrie matérialisée en buffers et ses bounds
//! - `lighting.rs` — lumières, ambiante, environnement, exposition,
//!   réglages d'ombre
//! - `passes.rs` — le registre des passes déclarées et leurs files
//! - `debug_draws.rs` — le store du debug rendering et sa résolution
//! - `instrumentation.rs` — l'analyse des draws, la clôture des
//!   diagnostics, le budget CPU
//! - `tests/` — les tests white-box, par domaine, dans l'ordre
//!   historique des sous-phases

mod debug_draws;
mod instrumentation;
mod lighting;
mod materials;
mod meshes;
mod passes;
mod pipelines;
mod resolve;
mod resources;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chaos_core::math::{Aabb, Mat4, Vec3, normal_matrix};
use chaos_core::{ChaosError, ChaosResult, Color};
use log::{debug, info, warn};

use crate::backend::{GraphicsBackend, create_backend};
use crate::capabilities::{CapabilityStatus, RendererCapabilities};
use crate::config::RendererConfig;
use crate::debug::{DebugDepth, DebugDraw, DebugStats};
use crate::diagnostics::{
    CpuCost, FallbackStats, FrameTotals, PassStats, RendererDiagnostics,
    ShadowStats as ShadowDiagnostics,
};
use crate::environment::{EnvironmentDescriptor, EnvironmentInfo};
use crate::frame::{
    DrawCommand, FrameDebugBatch, FrameDraw, FrameEnvironment, FrameOutcome, FramePass, FramePlan,
    FrameShadowPass, InstanceRange, InstanceTransforms, RenderDestination,
};
use crate::geometry::{Geometry, LitGeometry, TexturedGeometry};
use crate::lifetime::{
    KindStats, LifetimeTracker, RenderTargetInfo, ResourceStats, Retired, TextureInfo,
};
use crate::light::{FrameLights, Light, MAX_LIGHTS};
use crate::material::{
    MaterialDescriptor, MaterialHandle, MaterialInfo, MaterialModel, MaterialOpacity,
    MaterialRecord,
};
use crate::mesh::{MeshHandle, MeshRecord};
use crate::pass::{
    DrawBreakdown, FrameReport, PassHandle, PassLoad, PassOutcome, PassReport,
    RenderPassDescriptor, ShadowReport,
};
use crate::pool::{PoolHandle, ResourcePool};
use crate::queue::RenderQueue;
use crate::resources::{
    BufferDescriptor, BufferHandle, BuiltinTexture, ColorVertex, CullMode, DebugVertex,
    DepthCompare, LitVertex, MaterialBindingDescriptor, MaterialParams, PipelineDescriptor,
    PipelineHandle, PrimitiveTopology, RenderTargetDescriptor, RenderTargetHandle,
    SamplerDescriptor, SamplerHandle, ShaderRef, TextureDescriptor, TextureFormat, TextureHandle,
    TextureKind, TexturedVertex, VertexAttributeFormat, VertexLayout, instance_transforms_layout,
};
use crate::shaders::{ShaderLibrary, builtin};
use crate::shadow::{
    DirectionalShadowDescriptor, DirectionalShadowInfo, ShadowConfig, light_view_projection,
};
use crate::target::SurfaceTarget;
use crate::visibility::Frustum;

/// Renderer du moteur : orchestre un backend graphique interchangeable et
/// tient le registre des meshes (géométrie résidente GPU).
///
/// L'API ne parle que le vocabulaire de chaos_core ; le backend concret
/// (wgpu aujourd'hui) reste un détail d'implémentation interne.
pub struct Renderer {
    backend: Box<dyn GraphicsBackend>,
    shaders: ShaderLibrary,
    meshes: ResourcePool<MeshRecord>,
    materials: ResourcePool<MaterialRecord>,
    lifetime: LifetimeTracker,
    texture_cache: HashMap<String, TextureHandle>,
    fallback_sampler: Option<SamplerHandle>,
    surface_size: (u32, u32),
    passes: Vec<PassRecord>,
    report: FrameReport,
    last_clear_color: Color,
    pipeline_cache: HashMap<PipelineKey, PipelineHandle>,
    sky_pipelines: HashMap<Option<TextureFormat>, Option<PipelineHandle>>,
    shadow_pipelines: HashMap<(VertexLayout, bool, bool), Option<PipelineHandle>>,
    instanced_pipelines: HashMap<PipelineKey, Option<PipelineHandle>>,
    frame_lights: Vec<Light>,
    ambient_color: Color,
    ambient_intensity: f32,
    lights_truncation_warned: bool,
    environment: Option<EnvironmentState>,
    exposure: f32,
    directional_shadow: Option<DirectionalShadowDescriptor>,
    debug: DebugStore,
    debug_pipelines: HashMap<(Option<TextureFormat>, DebugDepth), Option<PipelineHandle>>,
    diagnostics: RendererDiagnostics,
    capabilities: RendererCapabilities,
}

/// L'état de l'environnement ACTIF — le miroir du descripteur accepté.
struct EnvironmentState {
    cubemap: TextureHandle,
    intensity: f32,
    sky: bool,
}

/// Le STORE du debug rendering : les primitives de la FRAME de
/// simulation (durée 0 — vidées par `clear_draws`, comme les draws) et
/// les RETENUES (durée > 0 — décomptées par `advance_debug_time`,
/// expirées seules), plus les toggles d'activation. Les toggles
/// filtrent au RENDU : une catégorie désactivée continue d'expirer.
struct DebugStore {
    enabled: bool,
    disabled_categories: HashSet<String>,
    frame: Vec<DebugDraw>,
    retained: Vec<RetainedDebugDraw>,
}

/// Une primitive RETENUE et son temps de vie restant.
struct RetainedDebugDraw {
    draw: DebugDraw,
    remaining: f32,
}

/// Le résultat de l'analyse d'une liste de draws résolus — la matière
/// des diagnostics par passe.
#[derive(Default)]
struct DrawAnalysis {
    classic_draws: usize,
    instanced_draws: usize,
    instances: usize,
    triangles: usize,
    debug_segments: usize,
    pipeline_switches: usize,
    material_switches: usize,
}

impl Default for DebugStore {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_categories: HashSet::new(),
            frame: Vec::new(),
            retained: Vec::new(),
        }
    }
}

/// La clé d'une permutation de pipeline material : le modèle, l'état de
/// rendu et le format de la destination (None = la surface). Les
/// paramètres (couleur, textures) n'y entrent pas — ils vivent dans le
/// binding. `None ≠ Some(format_surface)` même si identiques en
/// pratique : le renderer ne connaît pas le format de la surface
/// (dédup non parfaite, assumée).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PipelineKey {
    model: MaterialModel,
    double_sided: bool,
    opacity: MaterialOpacity,
    color_format: Option<TextureFormat>,
    instanced: bool,
}

/// L'identité de REGROUPEMENT d'un draw : le couple (material, mesh) —
/// deux draws consécutifs à clé égale partagent pipeline, binding et
/// buffers, et fusionnent en un draw instancié (à partir de 2).
type BatchKey = (u32, u32);

/// Ce que la fusion d'un run doit savoir de son groupe : la matière de
/// la permutation INSTANCIÉE — retenue une fois par (material, mesh) au
/// resolve.
#[derive(Clone)]
struct BatchGroup {
    model: MaterialModel,
    double_sided: bool,
    opacity: MaterialOpacity,
}

/// La MOISSON des casters d'ombre d'une frame : les draws (union des
/// passes actives), leur clé de regroupement et la matière des
/// permutations d'ombre instanciées — regroupée APRÈS la boucle des
/// passes (les duplicatas multi-passes fusionnent aussi). La moisson a
/// SA visibilité : le frustum de la LUMIÈRE (jamais celui d'une passe —
/// un caster hors caméra projette encore une ombre visible) ; `culled`
/// compte ses rejets.
#[derive(Default)]
struct ShadowHarvest {
    draws: Vec<FrameDraw>,
    keys: Vec<BatchKey>,
    groups: HashMap<BatchKey, (VertexLayout, bool)>,
    culled: usize,
}

/// Le résultat de la résolution d'UNE passe : les draws prêts pour le
/// backend, la ventilation par catégorie, les transforms d'instances et
/// le compte des objets REJETÉS par le frustum de la passe.
struct ResolvedPass {
    draws: Vec<FrameDraw>,
    breakdown: DrawBreakdown,
    instances: Vec<InstanceTransforms>,
    culled: usize,
}

/// L'index de la passe principale `chaos.main` — créée à la
/// construction, toujours présente.
const MAIN_PASS: usize = 0;

/// Une passe déclarée du registre : son descripteur et SA file de draws.
struct PassRecord {
    descriptor: RenderPassDescriptor,
    queue: RenderQueue,
}

/// Les organes de la RÉSOLUTION DE PIPELINES empruntés ensemble — des
/// champs disjoints déstructurés du Renderer, pour que les registres
/// (materials, meshes) et les files des passes restent lisibles à côté
/// pendant que le cache et le backend créent les permutations manquantes.
struct PipelineContext<'a> {
    pipeline_cache: &'a mut HashMap<PipelineKey, PipelineHandle>,
    sky_pipelines: &'a mut HashMap<Option<TextureFormat>, Option<PipelineHandle>>,
    shadow_pipelines: &'a mut HashMap<(VertexLayout, bool, bool), Option<PipelineHandle>>,
    instanced_pipelines: &'a mut HashMap<PipelineKey, Option<PipelineHandle>>,
    debug_pipelines: &'a mut HashMap<(Option<TextureFormat>, DebugDepth), Option<PipelineHandle>>,
    backend: &'a mut dyn GraphicsBackend,
    shaders: &'a ShaderLibrary,
    lifetime: &'a mut LifetimeTracker,
}

impl Renderer {
    /// Attache le renderer à une cible de présentation et initialise le GPU.
    pub fn attach(
        target: impl SurfaceTarget + 'static,
        config: RendererConfig,
    ) -> ChaosResult<Self> {
        let backend = create_backend(Box::new(target), config)?;
        info!("renderer ready: {}", backend.description());
        let mut renderer = Self::with_backend(backend);
        renderer.surface_size = (config.width, config.height);
        // Les capacités à l'attach : la ligne compacte dit les écarts
        // (les domaines HORS `Active`), le rapport complet part en debug.
        let degraded: Vec<String> = renderer
            .capabilities
            .decisions
            .iter()
            .filter(|decision| decision.status != CapabilityStatus::Active)
            .map(|decision| format!("{}: {}", decision.domain, decision.status))
            .collect();
        if degraded.is_empty() {
            info!(
                "gpu capabilities: all domains active ({} on {})",
                renderer.capabilities.backend, renderer.capabilities.adapter
            );
        } else {
            info!("gpu capabilities: {}", degraded.join(" | "));
        }
        debug!("{}", renderer.capabilities);
        Ok(renderer)
    }

    pub(crate) fn with_backend(backend: Box<dyn GraphicsBackend>) -> Self {
        let capabilities = backend.capabilities();
        Self {
            backend,
            shaders: ShaderLibrary::with_builtins(),
            meshes: ResourcePool::new(),
            materials: ResourcePool::new(),
            lifetime: LifetimeTracker::default(),
            texture_cache: HashMap::new(),
            fallback_sampler: None,
            surface_size: (1, 1),
            passes: vec![PassRecord {
                descriptor: RenderPassDescriptor::new("chaos.main", RenderDestination::Surface),
                queue: RenderQueue::new(),
            }],
            report: FrameReport::default(),
            last_clear_color: Color::BLACK,
            pipeline_cache: HashMap::new(),
            sky_pipelines: HashMap::new(),
            shadow_pipelines: HashMap::new(),
            instanced_pipelines: HashMap::new(),
            frame_lights: Vec::new(),
            ambient_color: Color::BLACK,
            ambient_intensity: 0.0,
            lights_truncation_warned: false,
            environment: None,
            exposure: 1.0,
            directional_shadow: None,
            debug: DebugStore::default(),
            debug_pipelines: HashMap::new(),
            diagnostics: RendererDiagnostics::default(),
            capabilities,
        }
    }

    /// LE rapport des capacités : ce que le GPU et la plateforme
    /// OFFRENT, ce que le renderer en a DÉCIDÉ et pourquoi — capturé à
    /// l'initialisation, STATIQUE (le pendant des diagnostics par
    /// frame). Les limites rapportées sont celles que le renderer FAIT
    /// RESPECTER avant le backend ; s'affiche en lignes de log
    /// lisibles (`Display`).
    pub fn capabilities(&self) -> &RendererCapabilities {
        &self.capabilities
    }

    /// La description humaine du backend actif (adaptateur, API) — logs.
    pub fn description(&self) -> String {
        self.backend.description()
    }

    /// La bibliothèque de shaders, en lecture.
    pub fn shaders(&self) -> &ShaderLibrary {
        &self.shaders
    }

    /// La bibliothèque de shaders, mutable — LE point d'extension des
    /// materials et des jeux (enregistrement de sources).
    pub fn shaders_mut(&mut self) -> &mut ShaderLibrary {
        &mut self.shaders
    }

    /// Fixe la couleur d'effacement de la passe principale — son load
    /// devient `Clear(color)` (le nom promet un effacement).
    pub fn set_clear_color(&mut self, color: Color) {
        self.passes[MAIN_PASS].descriptor.load = PassLoad::Clear(color);
        self.last_clear_color = color;
    }

    /// La couleur d'effacement courante de la passe principale — la
    /// dernière posée si son load est passé en `Keep`.
    pub fn clear_color(&self) -> Color {
        match self.passes[MAIN_PASS].descriptor.load {
            PassLoad::Clear(color) => color,
            PassLoad::Keep => self.last_clear_color,
        }
    }

    /// Fixe la caméra de la passe principale (la matrice vue-projection
    /// fournie par la caméra du moteur).
    pub fn set_view_projection(&mut self, view_projection: Mat4) {
        self.passes[MAIN_PASS].descriptor.view_projection = view_projection;
    }

    /// Fixe la position monde de la caméra de la passe principale —
    /// consommée par le spéculaire PBR (`Vec3::ZERO` si jamais posée :
    /// les modèles non-PBR ne la lisent pas).
    pub fn set_camera_position(&mut self, camera_position: Vec3) {
        self.passes[MAIN_PASS].descriptor.camera_position = camera_position;
    }

    /// Redimensionne la surface. Une dimension nulle (minimisation)
    /// suspend le rendu côté backend au lieu de reconfigurer — la
    /// dernière taille réelle est conservée.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_size = (width, height);
        }
        self.backend.resize(width, height);
    }

    /// Dernière taille de surface connue (largeur, hauteur) en pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        self.surface_size
    }

    /// Exécute la frame ORCHESTRÉE : les passes déclarées, dans l'ordre
    /// déterministe (tri stable par ordre puis enregistrement), chacune
    /// avec les draws résolus de SA file (materials → pipeline + binding,
    /// meshes → buffers ; une ressource détruite entre-temps est écartée
    /// avec un warn). Une passe désactivée est sautée ; une passe dont la
    /// cible est périmée s'AUTO-DÉSACTIVE avec un warn unique
    /// (`update_pass` la rebranche) ; un draw qui échantillonne la
    /// destination de sa propre passe est écarté (boucle de feedback).
    /// Le rapport (`frame_report`) est reconstruit, la retraite vidée au
    /// point sûr. Les draws restent en place jusqu'au prochain
    /// `clear_draws` du moteur.
    pub fn render_frame(&mut self) -> ChaosResult<FrameOutcome> {
        // Les diagnostics de la frame : le soumis capturé à l'ENTRÉE,
        // les coûts mesurés au fil de l'eau (~2 `Instant` par passe —
        // le coût de l'instrumentation est structurel, documenté).
        let frame_started = Instant::now();
        let submitted = self.draw_count();
        let mut pass_stats: Vec<PassStats> = Vec::new();
        let mut debug_segments = 0usize;
        let mut resolve_ms = 0.0f32;
        let mut schedule: Vec<usize> = (0..self.passes.len()).collect();
        schedule.sort_by_key(|&index| self.passes[index].descriptor.order);
        let mut plan_passes = Vec::new();
        let mut reports = Vec::new();
        // Les casters d'ombre : l'UNION des draws `cast_shadows` de
        // toutes les passes actives (les duplicatas multi-passes
        // FUSIONNENT au regroupement — la profondeur est idempotente).
        // Moissonnés SEULEMENT si des ombres sont configurées — sinon
        // aucun pipeline d'ombre n'est jamais créé.
        let mut shadow_casters = self
            .directional_shadow
            .is_some()
            .then(ShadowHarvest::default);
        // Les lumières se collectent AVANT la boucle (lecture pure) : la
        // vue de la LUMIÈRE — et donc SON frustum, la visibilité de la
        // moisson d'ombre — se précalcule une fois pour toutes les
        // passes, et se réutilise à la dérivation.
        let lights = self.collect_lights();
        let light_view = self.directional_shadow.as_ref().and_then(|settings| {
            lights.lights.iter().find_map(|light| match light {
                Light::Directional { direction, .. } => {
                    Some(light_view_projection(*direction, &settings.volume))
                }
                _ => None,
            })
        });
        let light_frustum = light_view.map(Frustum::from_view_projection);
        for index in schedule {
            let (label, destination) = {
                let descriptor = &self.passes[index].descriptor;
                (descriptor.label.clone(), descriptor.destination)
            };
            if !self.passes[index].descriptor.enabled {
                reports.push(PassReport {
                    label,
                    destination,
                    draws: 0,
                    draw_calls: 0,
                    culled: 0,
                    breakdown: DrawBreakdown::default(),
                    outcome: PassOutcome::Disabled,
                });
                continue;
            }
            let (blocked_texture, color_format) = match destination {
                RenderDestination::Surface => (None, None),
                RenderDestination::Target(target) => {
                    let Some(info) = self.lifetime.render_target_info(target) else {
                        warn!(
                            "render pass '{label}' disabled: its destination target is stale or destroyed"
                        );
                        self.passes[index].descriptor.enabled = false;
                        reports.push(PassReport {
                            label,
                            destination,
                            draws: 0,
                            draw_calls: 0,
                            culled: 0,
                            breakdown: DrawBreakdown::default(),
                            outcome: PassOutcome::StaleTarget,
                        });
                        continue;
                    };
                    (Some(info.color), Some(info.format))
                }
            };
            // Le ciel ne couvre que les passes `Clear` : dans une passe
            // `Keep`, il repeindrait l'image conservée (la profondeur y
            // repart à 1.0) ou le fond d'une caméra étrangère.
            let sky = self
                .environment
                .as_ref()
                .is_some_and(|environment| environment.sky)
                && matches!(self.passes[index].descriptor.load, PassLoad::Clear(_));
            let camera_position = self.passes[index].descriptor.camera_position;
            // Chaque passe cull avec SON frustum — jamais celui de la
            // principale.
            let pass_frustum =
                Frustum::from_view_projection(self.passes[index].descriptor.view_projection);
            let resolve_started = Instant::now();
            let (resolved, debug_batches, debug_vertices, debug_primitives) = {
                let Renderer {
                    passes,
                    materials,
                    meshes,
                    pipeline_cache,
                    sky_pipelines,
                    shadow_pipelines,
                    instanced_pipelines,
                    debug_pipelines,
                    debug,
                    backend,
                    shaders,
                    lifetime,
                    ..
                } = self;
                let mut context = PipelineContext {
                    pipeline_cache,
                    sky_pipelines,
                    shadow_pipelines,
                    instanced_pipelines,
                    debug_pipelines,
                    backend: backend.as_mut(),
                    shaders,
                    lifetime,
                };
                let resolved = Self::resolve_pass_draws(
                    materials,
                    meshes,
                    &mut context,
                    passes[index].queue.ordered(),
                    color_format,
                    blocked_texture,
                    sky,
                    camera_position,
                    &pass_frustum,
                    light_frustum.as_ref(),
                    shadow_casters.as_mut(),
                );
                // Le DEBUG de la passe, résolu APRÈS ses draws — ses
                // batches s'encodent derrière les transparents (le slot
                // réservé), l'overlay en dernier.
                let (debug_batches, debug_vertices, debug_primitives) =
                    Self::resolve_pass_debug(debug, &mut context, index, color_format);
                (resolved, debug_batches, debug_vertices, debug_primitives)
            };
            let ResolvedPass {
                draws,
                mut breakdown,
                instances,
                culled,
            } = resolved;
            // `draws` = les OBJETS logiques (la ventilation fait foi —
            // chaque primitive de debug dessinée est un objet INJECTÉ,
            // la règle du ciel) ; `draw_calls` = les soumissions
            // réelles, réduites par les draws instanciés, augmentées
            // des batches de debug.
            breakdown.injected += debug_primitives;
            let objects =
                breakdown.opaque + breakdown.masked + breakdown.transparent + breakdown.injected;
            // L'analyse de la passe : UNE itération sur les draws
            // RÉSOLUS (jamais les objets) — le détail des diagnostics.
            let pass_resolve_ms = resolve_started.elapsed().as_secs_f32() * 1000.0;
            resolve_ms += pass_resolve_ms;
            let analysis = Self::analyze_draws(&draws, &debug_batches);
            debug_segments += analysis.debug_segments;
            pass_stats.push(PassStats {
                label: label.clone(),
                draws: objects,
                draw_calls: draws.len() + debug_batches.len(),
                culled,
                triangles: analysis.triangles,
                classic_draws: analysis.classic_draws,
                instanced_draws: analysis.instanced_draws,
                instances: analysis.instances,
                pipeline_switches: analysis.pipeline_switches,
                material_switches: analysis.material_switches,
                resolve_cpu_ms: pass_resolve_ms,
            });
            reports.push(PassReport {
                label: label.clone(),
                destination,
                draws: objects,
                draw_calls: draws.len() + debug_batches.len(),
                culled,
                breakdown,
                outcome: PassOutcome::Executed,
            });
            plan_passes.push(FramePass {
                label,
                destination,
                load: self.passes[index].descriptor.load,
                view_projection: self.passes[index].descriptor.view_projection,
                camera_position: self.passes[index].descriptor.camera_position,
                draws,
                instances,
                debug: debug_batches,
                debug_vertices,
            });
        }
        if plan_passes.is_empty() {
            self.report = FrameReport {
                passes: reports,
                shadow: None,
            };
            self.flush_retired();
            // Le plan vide a AUSSI son snapshot — rien n'est parti au
            // backend : aucun événement de surface n'est compté.
            self.finish_diagnostics(
                submitted,
                pass_stats,
                None,
                debug_segments,
                resolve_ms,
                0.0,
                frame_started,
                None,
            );
            return Ok(FrameOutcome::Rendered);
        }
        // Le regroupement des casters se joue APRÈS la boucle des
        // passes : la moisson entière fusionne, duplicatas compris. Le
        // compte des rejets du frustum de lumière part au rapport.
        let harvest_started = Instant::now();
        let mut shadow_culled = 0;
        let harvested = shadow_casters.map(|harvest| {
            shadow_culled = harvest.culled;
            let Renderer {
                pipeline_cache,
                sky_pipelines,
                shadow_pipelines,
                instanced_pipelines,
                debug_pipelines,
                backend,
                shaders,
                lifetime,
                ..
            } = self;
            let mut context = PipelineContext {
                pipeline_cache,
                sky_pipelines,
                shadow_pipelines,
                instanced_pipelines,
                debug_pipelines,
                backend: backend.as_mut(),
                shaders,
                lifetime,
            };
            Self::batch_shadow_casters(&mut context, harvest)
        });
        let shadow = self.derive_shadow_pass(&lights, light_view, harvested);
        resolve_ms += harvest_started.elapsed().as_secs_f32() * 1000.0;
        let shadow_report = shadow.as_ref().map(|shadow| ShadowReport {
            draws: shadow
                .draws
                .iter()
                .map(|draw| draw.instances.map_or(1, |range| range.count as usize))
                .sum(),
            draw_calls: shadow.draws.len(),
            culled: shadow_culled,
            resolution: shadow.resolution,
        });
        // Les diagnostics de l'ombre : la même analyse que les passes.
        let shadow_stats =
            shadow
                .as_ref()
                .zip(shadow_report.as_ref())
                .map(|(shadow_pass, report)| {
                    let analysis = Self::analyze_draws(&shadow_pass.draws, &[]);
                    ShadowDiagnostics {
                        draws: report.draws,
                        draw_calls: report.draw_calls,
                        culled: report.culled,
                        instances: analysis.instances,
                        triangles: analysis.triangles,
                    }
                });
        let plan = FramePlan {
            passes: plan_passes,
            lights,
            environment: self.frame_environment(),
            shadow,
        };
        let backend_started = Instant::now();
        let outcome = self.backend.render(&plan)?;
        let backend_ms = backend_started.elapsed().as_secs_f32() * 1000.0;
        if matches!(outcome, FrameOutcome::Skipped(_)) {
            for report in &mut reports {
                if report.destination == RenderDestination::Surface
                    && report.outcome == PassOutcome::Executed
                {
                    report.outcome = PassOutcome::SurfaceSkipped;
                }
            }
        }
        self.report = FrameReport {
            passes: reports,
            shadow: shadow_report,
        };
        self.flush_retired();
        self.finish_diagnostics(
            submitted,
            pass_stats,
            shadow_stats,
            debug_segments,
            resolve_ms,
            backend_ms,
            frame_started,
            Some(outcome),
        );
        Ok(outcome)
    }

    /// Rend une passe dans une cible hors écran, IMMÉDIATEMENT et hors de
    /// la frame orchestrée (vignettes d'éditeur, bakes) : les draws
    /// fournis sont triés par material (copie locale — les files des
    /// passes déclarées ne sont pas touchées), résolus comme ceux de la
    /// frame (les périmés écartés pareil, le feedback aussi), puis
    /// exécutés vers la cible en un plan mono-passe `chaos.offscreen` —
    /// sans présentation, sans skip (une cible est toujours disponible,
    /// fenêtre minimisée comprise). Le résultat s'exploite ensuite via
    /// `render_target_color` ; le rapport de frame n'est pas touché.
    pub fn render_to_target(
        &mut self,
        target: RenderTargetHandle,
        clear_color: Color,
        view_projection: Mat4,
        commands: &[DrawCommand],
    ) -> ChaosResult<FrameOutcome> {
        let Some(info) = self.lifetime.render_target_info(target) else {
            return Err(ChaosError::Graphics(String::from(
                "render target handle is stale or already destroyed",
            )));
        };
        let blocked_texture = Some(info.color);
        let color_format = Some(info.format);
        let mut ordered = commands.to_vec();
        ordered.sort_by_key(|command| command.material.index);
        // La passe offscreen efface toujours sa cible : le ciel s'y
        // dessine dès que l'environnement l'active.
        let sky = self
            .environment
            .as_ref()
            .is_some_and(|environment| environment.sky);
        // La vignette cull avec SA vue-projection — chaque vue possède
        // sa visibilité, le chemin immédiat compris.
        let pass_frustum = Frustum::from_view_projection(view_projection);
        let resolved = {
            let Renderer {
                materials,
                meshes,
                pipeline_cache,
                sky_pipelines,
                shadow_pipelines,
                instanced_pipelines,
                debug_pipelines,
                backend,
                shaders,
                lifetime,
                ..
            } = self;
            let mut context = PipelineContext {
                pipeline_cache,
                sky_pipelines,
                shadow_pipelines,
                instanced_pipelines,
                debug_pipelines,
                backend: backend.as_mut(),
                shaders,
                lifetime,
            };
            Self::resolve_pass_draws(
                materials,
                meshes,
                &mut context,
                &ordered,
                color_format,
                blocked_texture,
                sky,
                // Le chemin immédiat n'a pas de position caméra (sa
                // limite documentée) : le tri part de l'origine —
                // déterministe, jamais faux pour une vignette fixe.
                Vec3::ZERO,
                &pass_frustum,
                None,
                None,
            )
        };
        let (draws, instances) = (resolved.draws, resolved.instances);
        let plan = FramePlan {
            passes: vec![FramePass {
                label: String::from("chaos.offscreen"),
                destination: RenderDestination::Target(target),
                load: PassLoad::Clear(clear_color),
                view_projection,
                camera_position: Vec3::ZERO,
                draws,
                instances,
                // Le chemin immédiat ne dessine PAS de debug — la règle
                // de la passe d'ombre (documenté).
                debug: Vec::new(),
                debug_vertices: Vec::new(),
            }],
            lights: self.collect_lights(),
            environment: self.frame_environment(),
            // Le chemin immédiat ne rend PAS de passe d'ombre : ses
            // draws échantillonnent la map du dernier plan (documenté).
            shadow: None,
        };
        self.backend.render(&plan)
    }
}
