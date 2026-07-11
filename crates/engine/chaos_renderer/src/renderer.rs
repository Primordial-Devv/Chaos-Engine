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

    /// Fixe la lumière AMBIANTE — un RÉGLAGE persistant (le patron de
    /// `set_clear_color`), jamais vidé par `clear_draws`. Le défaut est
    /// (noir, 0) : sans ambiante ni lumière, une surface éclairée est
    /// noire.
    pub fn set_ambient_light(&mut self, color: Color, intensity: f32) {
        self.ambient_color = color;
        self.ambient_intensity = intensity;
    }

    /// La lumière ambiante courante (couleur, intensité).
    pub fn ambient_light(&self) -> (Color, f32) {
        (self.ambient_color, self.ambient_intensity)
    }

    /// Fixe l'ENVIRONNEMENT de la scène — un RÉGLAGE persistant (le
    /// patron de l'ambiante), jamais vidé par `clear_draws`. La cubemap
    /// doit être vivante et de kind `Cube` ; l'intensité finie, positive
    /// ou nulle. Re-poser le MÊME cubemap ne rebinde pas le backend
    /// (mise à jour intensité/ciel seule). Backend d'abord, état
    /// ensuite : un refus laisse l'environnement précédent intact.
    pub fn set_environment(&mut self, descriptor: &EnvironmentDescriptor) -> ChaosResult<()> {
        let Some(info) = self.lifetime.texture_info(descriptor.cubemap) else {
            return Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            )));
        };
        if info.kind != TextureKind::Cube {
            return Err(ChaosError::Graphics(format!(
                "environment: texture '{}' is a {:?} texture — the environment expects a cubemap",
                info.label, info.kind
            )));
        }
        if !descriptor.intensity.is_finite() || descriptor.intensity < 0.0 {
            return Err(ChaosError::Graphics(format!(
                "environment '{}': intensity must be finite and non-negative, got {}",
                info.label, descriptor.intensity
            )));
        }
        if self.environment.as_ref().map(|env| env.cubemap) != Some(descriptor.cubemap) {
            self.backend.set_environment(Some(descriptor.cubemap))?;
        }
        self.environment = Some(EnvironmentState {
            cubemap: descriptor.cubemap,
            intensity: descriptor.intensity,
            sky: descriptor.sky,
        });
        Ok(())
    }

    /// Efface l'environnement : le backend rebinde son cube fallback
    /// noir (contribution nulle, sans branche shader), le ciel
    /// disparaît. Idempotent — effacer sans environnement est un no-op.
    pub fn clear_environment(&mut self) -> ChaosResult<()> {
        if self.environment.is_some() {
            self.backend.set_environment(None)?;
            self.environment = None;
        }
        Ok(())
    }

    /// L'inspection de l'environnement actif, s'il existe — le miroir
    /// lisible de l'état pour les outils.
    pub fn environment_info(&self) -> Option<EnvironmentInfo> {
        let environment = self.environment.as_ref()?;
        let info = self.lifetime.texture_info(environment.cubemap)?;
        Some(EnvironmentInfo {
            label: info.label.clone(),
            intensity: environment.intensity,
            sky: environment.sky,
            mip_levels: info.mip_levels,
        })
    }

    /// Configure les ombres de la lumière directionnelle principale — un
    /// réglage PERSISTANT (le patron de l'environnement), jamais vidé par
    /// `clear_draws`. La lumière qui projette est la PREMIÈRE
    /// directionnelle activée et valide de chaque frame ; sans
    /// directionnelle, la passe d'ombre est simplement absente. Le
    /// descripteur est VALIDÉ avant tout appel backend ; le backend n'est
    /// touché que si la RÉSOLUTION change (la map est recréée) — volume
    /// et biais sont des données par frame, réglables à chaud sans
    /// recréation.
    pub fn set_directional_shadow(
        &mut self,
        descriptor: &DirectionalShadowDescriptor,
    ) -> ChaosResult<()> {
        descriptor.validate()?;
        // La borne DEVICE, distincte de la borne engine (16..=8192 du
        // descripteur) : le message nomme LA limite qui refuse.
        let device_ceiling = self.capabilities.limits.max_texture_2d;
        if descriptor.resolution > device_ceiling {
            return Err(ChaosError::Graphics(format!(
                "shadow map resolution {} exceeds the device texture limit ({device_ceiling})",
                descriptor.resolution
            )));
        }
        if self.directional_shadow.map(|current| current.resolution) != Some(descriptor.resolution)
        {
            self.backend.set_shadow(Some(ShadowConfig {
                resolution: descriptor.resolution,
            }))?;
        }
        self.directional_shadow = Some(*descriptor);
        Ok(())
    }

    /// Efface les ombres : la shadow map backend est libérée, le
    /// fallback « tout éclairé » rebindé — plus aucune atténuation.
    /// Idempotent — effacer sans ombres est un no-op.
    pub fn clear_directional_shadow(&mut self) -> ChaosResult<()> {
        if self.directional_shadow.is_some() {
            self.backend.set_shadow(None)?;
            self.directional_shadow = None;
        }
        Ok(())
    }

    /// L'inspection des ombres configurées, s'il y en a — le miroir
    /// lisible de l'état pour les outils (résolution, volume, biais).
    pub fn directional_shadow_info(&self) -> Option<DirectionalShadowInfo> {
        self.directional_shadow
            .as_ref()
            .map(|settings| DirectionalShadowInfo {
                resolution: settings.resolution,
                volume: settings.volume,
                depth_bias: settings.depth_bias,
                normal_bias: settings.normal_bias,
            })
    }

    /// Fixe l'EXPOSITION globale, appliquée avant le tone mapping (les
    /// chemins tone-mappés : PBR et ciel — les modèles Unlit/Lit ne la
    /// lisent pas) — un réglage persistant, 1.0 par défaut. Refus
    /// explicite d'une valeur non finie ou non strictement positive.
    pub fn set_exposure(&mut self, exposure: f32) -> ChaosResult<()> {
        if !exposure.is_finite() || exposure <= 0.0 {
            return Err(ChaosError::Graphics(format!(
                "exposure must be a positive, finite value, got {exposure}"
            )));
        }
        self.exposure = exposure;
        Ok(())
    }

    /// L'exposition globale courante.
    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    /// L'environnement de frame RÉSOLU pour le plan : l'intensité active
    /// (0 sans environnement) et l'exposition globale.
    fn frame_environment(&self) -> FrameEnvironment {
        FrameEnvironment {
            intensity: self.environment.as_ref().map_or(0.0, |env| env.intensity),
            exposure: self.exposure,
        }
    }

    /// Soumet une lumière pour la frame de simulation courante — le
    /// pendant lumineux de `queue_draw` : re-soumise chaque frame, vidée
    /// par `clear_draws`. Une lumière INVALIDE (direction nulle,
    /// intensité négative, cône dégénéré, NaN) est écartée ici avec un
    /// warn — jamais envoyée au GPU. Au-delà de [`MAX_LIGHTS`] lumières
    /// activées, les premières soumises gagnent (troncature prévisible,
    /// un warn par épisode).
    pub fn submit_light(&mut self, light: Light) {
        if let Some(reason) = light.invalid_reason() {
            warn!("light dropped: {reason}");
            return;
        }
        self.frame_lights.push(light);
    }

    /// La COLLECTION d'éclairage de la frame — le chemin partagé de
    /// `render_frame` et `render_to_target` : filtre les lumières
    /// désactivées, normalise les directions, tronque à [`MAX_LIGHTS`]
    /// (les premières soumises gagnent) avec un warn par épisode de
    /// dépassement.
    fn collect_lights(&mut self) -> FrameLights {
        let enabled = self.frame_lights.iter().filter(|light| light.is_enabled());
        let lights: Vec<Light> = enabled
            .clone()
            .take(MAX_LIGHTS)
            .map(Light::normalized)
            .collect();
        let submitted = enabled.count();
        if submitted > MAX_LIGHTS {
            if !self.lights_truncation_warned {
                warn!(
                    "light overflow: {submitted} enabled lights submitted, only the first {MAX_LIGHTS} are kept"
                );
                self.lights_truncation_warned = true;
            }
        } else {
            self.lights_truncation_warned = false;
        }
        FrameLights {
            ambient_color: self.ambient_color,
            ambient_intensity: self.ambient_intensity,
            lights,
        }
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
    fn create_pipeline_with(
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
    fn resolve_material_pipeline(
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
    fn resolve_instanced_pipeline(
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
    fn resolve_sky_pipeline(
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
    fn resolve_debug_pipeline(
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

    /// Résout le DEBUG d'une passe : les primitives visibles (toggle
    /// global, catégorie, passe cible — `None` vise la principale) sont
    /// tessellées en DEUX plages d'un même tableau de sommets — Scene
    /// (testée) puis Overlay (par-dessus tout, dessinée en dernier) —
    /// et chaque plage non vide devient un batch avec sa permutation.
    /// Rend (batches, sommets, primitives DESSINÉES) — le compte nourrit
    /// `injected` (la règle du ciel).
    fn resolve_pass_debug(
        debug: &DebugStore,
        context: &mut PipelineContext<'_>,
        pass_index: usize,
        color_format: Option<TextureFormat>,
    ) -> (Vec<FrameDebugBatch>, Vec<DebugVertex>, usize) {
        if !debug.enabled {
            return (Vec::new(), Vec::new(), 0);
        }
        let mut scene = Vec::new();
        let mut overlay = Vec::new();
        let mut scene_count = 0usize;
        let mut overlay_count = 0usize;
        let draws = debug
            .frame
            .iter()
            .chain(debug.retained.iter().map(|retained| &retained.draw));
        for draw in draws {
            if debug.disabled_categories.contains(&draw.category) {
                continue;
            }
            let target = draw.pass.map_or(MAIN_PASS, |pass| pass.0 as usize);
            if target != pass_index {
                continue;
            }
            match draw.depth {
                DebugDepth::Scene => {
                    draw.shape.tessellate(draw.color, &mut scene);
                    scene_count += 1;
                }
                DebugDepth::Overlay => {
                    draw.shape.tessellate(draw.color, &mut overlay);
                    overlay_count += 1;
                }
            }
        }
        let mut batches = Vec::new();
        let mut vertices = Vec::new();
        let mut primitives = 0;
        for (mode_vertices, count, depth) in [
            (scene, scene_count, DebugDepth::Scene),
            (overlay, overlay_count, DebugDepth::Overlay),
        ] {
            if mode_vertices.is_empty() {
                continue;
            }
            let Some(pipeline) = Self::resolve_debug_pipeline(context, color_format, depth) else {
                continue;
            };
            let first_vertex = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            let vertex_count = u32::try_from(mode_vertices.len()).unwrap_or(u32::MAX);
            vertices.extend(mode_vertices);
            batches.push(FrameDebugBatch {
                pipeline,
                first_vertex,
                vertex_count,
            });
            primitives += count;
        }
        (batches, vertices, primitives)
    }

    /// Résout la permutation du pipeline d'OMBRE pour un vertex layout et
    /// un état de culling — un cache dédié (l'ombre n'est pas un
    /// material : profondeur seule, vertex uniquement, groupe(0) réduit).
    /// Un layout SANS position (`Float32x3` à la location 0) ou un échec
    /// de création est MÉMOÏSÉ avec un warn unique par permutation : le
    /// caster est écarté, jamais la frame.
    fn resolve_shadow_pipeline(
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
    fn check_buffer_limit(&self, label: &str, bytes: usize) -> ChaosResult<()> {
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

    /// Texture de repli des materials sans texture (`chaos.white`).
    fn fallback_texture(&mut self) -> ChaosResult<TextureHandle> {
        self.builtin_texture(BuiltinTexture::White)
    }

    /// Sampler de repli builtin (`chaos.default_sampler`, Linear + Repeat)
    /// — PROTÉGÉ : le détruire est un refus explicite.
    fn fallback_sampler(&mut self) -> ChaosResult<SamplerHandle> {
        if let Some(sampler) = self.fallback_sampler {
            return Ok(sampler);
        }
        let sampler =
            self.create_sampler_tracked(&SamplerDescriptor::new("chaos.default_sampler"), true)?;
        self.fallback_sampler = Some(sampler);
        Ok(sampler)
    }

    /// Crée un mesh à sommets colorés : téléverse la géométrie (vertex +
    /// index buffers) et l'enregistre comme ressource de rendu. Le mesh
    /// possède ses buffers.
    pub fn create_mesh(&mut self, label: &str, geometry: &Geometry) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            ColorVertex::layout(),
            bounds,
        )
    }

    /// Crée un mesh à sommets texturés (position + UV) — même cycle de vie
    /// que `create_mesh`, layout `TexturedVertex`.
    pub fn create_textured_mesh(
        &mut self,
        label: &str,
        geometry: &TexturedGeometry,
    ) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            TexturedVertex::layout(),
            bounds,
        )
    }

    /// Crée un mesh ÉCLAIRABLE (position + normale + UV) : téléverse la
    /// géométrie et l'enregistre comme ressource de rendu — le mesh des
    /// materials `Lit`.
    pub fn create_lit_mesh(
        &mut self,
        label: &str,
        geometry: &LitGeometry,
    ) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        let bounds = Self::geometry_bounds(
            label,
            geometry.vertices.iter().map(|vertex| vertex.position),
        );
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            LitVertex::layout(),
            bounds,
        )
    }

    /// Les BOUNDS locaux d'une géométrie : l'AABB de ses positions —
    /// `None` (géométrie vide ou position non finie, avec warn) = le
    /// mesh ne sera JAMAIS cullé, le défaut sûr.
    fn geometry_bounds(label: &str, positions: impl IntoIterator<Item = [f32; 3]>) -> Option<Aabb> {
        let mut empty = true;
        let bounds = Aabb::from_points(positions.into_iter().map(|position| {
            empty = false;
            Vec3::from(position)
        }));
        if bounds.is_none() && !empty {
            warn!("mesh '{label}' carries non-finite positions — it will never be culled");
        }
        bounds
    }

    fn register_mesh(
        &mut self,
        label: &str,
        vertex_bytes: Vec<u8>,
        index_bytes: Option<Vec<u8>>,
        element_count: u32,
        vertex_layout: VertexLayout,
        bounds: Option<Aabb>,
    ) -> ChaosResult<MeshHandle> {
        let vertex_descriptor = BufferDescriptor::vertex(label, vertex_bytes);
        // La borne device couvre AUSSI le chemin des meshes — le même
        // refus nommé que les buffers publics.
        self.check_buffer_limit(&vertex_descriptor.label, vertex_descriptor.contents.len())?;
        let vertex_buffer = self.backend.create_buffer(&vertex_descriptor)?;
        self.lifetime.register_buffer(
            vertex_buffer,
            &vertex_descriptor.label,
            vertex_descriptor.contents.len() as u64,
            Some(label),
        );
        let index_buffer = match index_bytes {
            Some(bytes) => {
                let index_descriptor = BufferDescriptor::index(format!("{label}.indices"), bytes);
                self.check_buffer_limit(&index_descriptor.label, index_descriptor.contents.len())?;
                let handle = self.backend.create_buffer(&index_descriptor)?;
                self.lifetime.register_buffer(
                    handle,
                    &index_descriptor.label,
                    index_descriptor.contents.len() as u64,
                    Some(label),
                );
                Some(handle)
            }
            None => None,
        };
        let record = MeshRecord {
            vertex_buffer,
            index_buffer,
            element_count,
            vertex_layout,
            bounds,
        };
        let stride = record.vertex_layout.stride;
        let pool_handle = self
            .meshes
            .insert(record)
            .ok_or_else(|| ChaosError::Graphics(String::from("mesh pool capacity exceeded")))?;
        let handle = MeshHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!("mesh '{label}' created ({element_count} elements, stride {stride}, {handle:?})");
        Ok(handle)
    }

    /// Les BOUNDS locaux d'un mesh vivant — `None` = le mesh n'en porte
    /// pas (géométrie vide ou dégénérée) et n'est jamais cullé.
    /// L'inspection du culling et du futur éditeur.
    pub fn mesh_bounds(&self, handle: MeshHandle) -> ChaosResult<Option<Aabb>> {
        self.meshes
            .get(PoolHandle {
                index: handle.index,
                generation: handle.generation,
            })
            .map(|record| record.bounds)
            .ok_or_else(|| {
                ChaosError::Graphics(String::from("mesh handle is stale or already destroyed"))
            })
    }

    /// Détruit un mesh : le propriétaire emporte ses buffers — ils partent
    /// en retraite (libération backend différée). Un handle périmé est une
    /// erreur explicite.
    pub fn destroy_mesh(&mut self, handle: MeshHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.meshes.remove(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "mesh handle is stale or already destroyed",
            )));
        };
        self.lifetime.retire_owned_buffer(record.vertex_buffer);
        if let Some(index_buffer) = record.index_buffer {
            self.lifetime.retire_owned_buffer(index_buffer);
        }
        debug!("mesh released ({handle:?})");
        Ok(())
    }

    /// Soumet un ordre de dessin à la passe principale pour la frame de
    /// simulation courante.
    pub fn queue_draw(&mut self, command: DrawCommand) {
        self.passes[MAIN_PASS].queue.submit(command);
    }

    /// Soumet un ordre de dessin à une passe déclarée. Une passe
    /// désactivée accepte ses draws (la file est vidée au prochain
    /// `clear_draws`, rien n'est rendu) ; un handle inconnu est une
    /// erreur explicite.
    pub fn queue_draw_to(&mut self, pass: PassHandle, command: DrawCommand) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.queue.submit(command);
        Ok(())
    }

    /// Vide les files de TOUTES les passes — appelée par le moteur au
    /// début de chaque frame de simulation. Les draws survivent ainsi aux
    /// présentations multiples entre deux updates (rafales de redraw du
    /// resize interactif).
    pub fn clear_draws(&mut self) {
        for record in &mut self.passes {
            record.queue.clear();
        }
        self.frame_lights.clear();
        // Le debug de FRAME suit les draws ; les RETENUES survivent —
        // c'est leur raison d'être (elles expirent par le temps).
        self.debug.frame.clear();
    }

    /// Le nombre de draws soumis pour la frame de simulation courante,
    /// TOUTES passes confondues — la jauge des metrics de santé.
    pub fn draw_count(&self) -> usize {
        self.passes.iter().map(|record| record.queue.len()).sum()
    }

    /// Soumet une primitive de DEBUG — le pendant visuel de
    /// `queue_draw` pour les données spatiales. Durée `0` (défaut) : la
    /// primitive vit la frame de simulation courante, vidée par
    /// `clear_draws` ; durée `> 0` : RETENUE, elle survit à
    /// `clear_draws` et expire seule au fil de `advance_debug_time`.
    /// Une primitive INVALIDE (géométrie non finie, taille nulle,
    /// frustum non inversible, passe inconnue…) est écartée ICI avec un
    /// warn — jamais stockée, jamais envoyée au GPU.
    pub fn queue_debug(&mut self, draw: DebugDraw) {
        if let Err(refusal) = draw.validate() {
            warn!("{refusal}");
            return;
        }
        if let Some(pass) = draw.pass
            && self.passes.get(pass.0 as usize).is_none()
        {
            warn!("debug draw refused: unknown render pass handle");
            return;
        }
        if draw.duration > 0.0 {
            self.debug.retained.push(RetainedDebugDraw {
                remaining: draw.duration,
                draw,
            });
        } else {
            self.debug.frame.push(draw);
        }
    }

    /// Avance le temps du debug rendering : les primitives RETENUES
    /// décomptent `delta_seconds` et expirent seules. Le renderer n'a
    /// pas d'horloge — le CONSOMMATEUR fournit le temps (la démo le
    /// fait à chaque update). Un delta invalide (non fini, négatif) est
    /// ignoré avec un warn.
    pub fn advance_debug_time(&mut self, delta_seconds: f32) {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            warn!("debug time ignored: delta must be finite and non-negative");
            return;
        }
        self.debug.retained.retain_mut(|retained| {
            retained.remaining -= delta_seconds;
            retained.remaining > 0.0
        });
    }

    /// Active ou désactive TOUT le debug rendering (activé par défaut).
    /// Les soumissions restent acceptées et les retenues continuent
    /// d'expirer — le toggle filtre au RENDU seulement.
    pub fn set_debug_enabled(&mut self, enabled: bool) {
        self.debug.enabled = enabled;
    }

    /// Le debug rendering est-il activé ?
    pub fn debug_enabled(&self) -> bool {
        self.debug.enabled
    }

    /// Active ou désactive une CATÉGORIE de debug (toutes activées par
    /// défaut). Le filtre agit au RENDU : les retenues d'une catégorie
    /// désactivée continuent d'expirer et réapparaissent au réveil.
    pub fn set_debug_category_enabled(&mut self, category: &str, enabled: bool) {
        if enabled {
            self.debug.disabled_categories.remove(category);
        } else {
            self.debug
                .disabled_categories
                .insert(String::from(category));
        }
    }

    /// La catégorie de debug est-elle activée ?
    pub fn debug_category_enabled(&self, category: &str) -> bool {
        !self.debug.disabled_categories.contains(category)
    }

    /// L'inspection du store de debug : les comptes frame/retenues.
    pub fn debug_stats(&self) -> DebugStats {
        DebugStats {
            frame: self.debug.frame.len(),
            retained: self.debug.retained.len(),
        }
    }

    /// Le handle de la passe principale `chaos.main` (surface, ordre 0) —
    /// créée à la construction, toujours présente. La désactiver est le
    /// mécanisme officiel du rendu tout-hors-écran.
    pub fn main_pass(&self) -> PassHandle {
        PassHandle(MAIN_PASS as u32)
    }

    /// Déclare une passe de rendu. La déclaration est VALIDÉE avant
    /// d'entrer au registre : label non vide, unique et hors du préfixe
    /// réservé `chaos.` ; destination et lectures vivantes ; pas de
    /// boucle de feedback déclarée ; et l'invariant d'ordonnancement —
    /// une passe qui écrit une cible lue par une autre doit s'exécuter
    /// AVANT la lectrice. Un refus nomme la règle et laisse le registre
    /// intact. Les passes sont permanentes en V1 : pas de suppression,
    /// la désactivation (`set_pass_enabled`) en tient lieu.
    pub fn add_pass(&mut self, descriptor: &RenderPassDescriptor) -> ChaosResult<PassHandle> {
        descriptor.validate()?;
        if descriptor.label.starts_with("chaos.") {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}': the 'chaos.' label prefix is reserved for engine passes",
                descriptor.label
            )));
        }
        if self
            .passes
            .iter()
            .any(|record| record.descriptor.label == descriptor.label)
        {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}' already exists: labels are unique",
                descriptor.label
            )));
        }
        self.check_pass_resources(descriptor)?;
        let mut candidates: Vec<&RenderPassDescriptor> = self
            .passes
            .iter()
            .map(|record| &record.descriptor)
            .collect();
        candidates.push(descriptor);
        self.validate_ordering(&candidates)?;
        let handle = PassHandle(self.passes.len() as u32);
        debug!(
            "render pass '{}' declared (order {}, {handle:?})",
            descriptor.label, descriptor.order
        );
        self.passes.push(PassRecord {
            descriptor: descriptor.clone(),
            queue: RenderQueue::new(),
        });
        Ok(handle)
    }

    /// Remplace le descripteur d'une passe déclarée — SA file de draws
    /// est conservée. Mêmes validations qu'à la déclaration, revalidées
    /// sur l'ENSEMBLE du registre (changer l'ordre d'une passe peut
    /// casser l'invariant entre deux autres) ; un refus laisse tout
    /// intact. C'est le chemin du redimensionnement d'une cible : le
    /// resize fait tourner le handle, `update_pass` rebranche la passe
    /// (et la réactive si elle s'était auto-désactivée). La passe
    /// principale est protégée : sa destination (la surface) et son
    /// label ne changent pas — load, caméra, ordre et activation restent
    /// libres.
    pub fn update_pass(
        &mut self,
        pass: PassHandle,
        descriptor: &RenderPassDescriptor,
    ) -> ChaosResult<()> {
        let index = pass.0 as usize;
        if self.passes.get(index).is_none() {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        }
        descriptor.validate()?;
        if index == MAIN_PASS {
            if descriptor.destination != RenderDestination::Surface {
                return Err(ChaosError::Graphics(String::from(
                    "the main pass renders to the surface: its destination cannot change",
                )));
            }
            if descriptor.label != self.passes[MAIN_PASS].descriptor.label {
                return Err(ChaosError::Graphics(String::from(
                    "the main pass label cannot change",
                )));
            }
        } else {
            if descriptor.label.starts_with("chaos.") {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}': the 'chaos.' label prefix is reserved for engine passes",
                    descriptor.label
                )));
            }
            if self.passes.iter().enumerate().any(|(other, record)| {
                other != index && record.descriptor.label == descriptor.label
            }) {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}' already exists: labels are unique",
                    descriptor.label
                )));
            }
        }
        self.check_pass_resources(descriptor)?;
        let candidates: Vec<&RenderPassDescriptor> = self
            .passes
            .iter()
            .enumerate()
            .map(|(other, record)| {
                if other == index {
                    descriptor
                } else {
                    &record.descriptor
                }
            })
            .collect();
        self.validate_ordering(&candidates)?;
        if index == MAIN_PASS
            && let PassLoad::Clear(color) = descriptor.load
        {
            self.last_clear_color = color;
        }
        self.passes[index].descriptor = descriptor.clone();
        debug!(
            "render pass '{}' updated (order {}, {pass:?})",
            descriptor.label, descriptor.order
        );
        Ok(())
    }

    /// Active ou désactive une passe — une passe désactivée est sautée
    /// proprement à chaque frame (visible au rapport), ses draws
    /// acceptés puis vidés sans être rendus.
    pub fn set_pass_enabled(&mut self, pass: PassHandle, enabled: bool) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.enabled = enabled;
        Ok(())
    }

    /// Remplace la caméra d'une passe (sa matrice vue-projection) — le
    /// réglage par frame des caméras dynamiques (ombres, reflets).
    pub fn set_pass_camera(&mut self, pass: PassHandle, view_projection: Mat4) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.view_projection = view_projection;
        Ok(())
    }

    /// Remplace la position monde de la caméra d'une passe — le
    /// spéculaire PBR de la passe.
    pub fn set_pass_camera_position(
        &mut self,
        pass: PassHandle,
        camera_position: Vec3,
    ) -> ChaosResult<()> {
        let Some(record) = self.passes.get_mut(pass.0 as usize) else {
            return Err(ChaosError::Graphics(String::from(
                "render pass handle is unknown",
            )));
        };
        record.descriptor.camera_position = camera_position;
        Ok(())
    }

    /// Le rapport de la dernière frame orchestrée — passe par passe dans
    /// l'ordre d'exécution. Vide avant la première frame ; reconstruit à
    /// chaque `render_frame` ; `render_to_target` n'y touche pas.
    pub fn frame_report(&self) -> &FrameReport {
        &self.report
    }

    /// LE snapshot des diagnostics du renderer : ce que la DERNIÈRE
    /// frame orchestrée a rendu, éliminé, possédé et coûté — compteurs
    /// exacts, coûts CPU mesurés, temps GPU honnête (`Unavailable` avec
    /// sa raison quand la mesure n'existe pas). Reconstruit à chaque
    /// `render_frame` (les champs de surface et de budget sont
    /// CUMULATIFS) ; `render_to_target` n'y touche pas. S'affiche en
    /// lignes de log lisibles (`Display`) — utilisable sans UI.
    pub fn diagnostics(&self) -> &RendererDiagnostics {
        &self.diagnostics
    }

    /// Fixe le BUDGET CPU par frame du renderer, en millisecondes —
    /// `None` (défaut) : jamais de dépassement. Chaque `render_frame`
    /// dont `total_ms` dépasse le budget incrémente
    /// `diagnostics().budget.over_budget_frames`. Une valeur invalide
    /// (non finie ou ≤ 0) est ignorée avec un warn.
    pub fn set_cpu_budget(&mut self, budget_ms: Option<f32>) {
        if let Some(budget) = budget_ms
            && (!budget.is_finite() || budget <= 0.0)
        {
            warn!("cpu budget ignored: it must be finite and positive");
            return;
        }
        self.diagnostics.budget.budget_ms = budget_ms;
    }

    /// Les cibles d'une passe doivent être vivantes à sa déclaration —
    /// destination comme lectures.
    fn check_pass_resources(&self, descriptor: &RenderPassDescriptor) -> ChaosResult<()> {
        if let RenderDestination::Target(target) = descriptor.destination
            && self.lifetime.render_target_info(target).is_none()
        {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}': its destination target is stale or already destroyed",
                descriptor.label
            )));
        }
        for read in &descriptor.reads {
            if self.lifetime.render_target_info(*read).is_none() {
                return Err(ChaosError::Graphics(format!(
                    "render pass '{}': a declared read is stale or already destroyed",
                    descriptor.label
                )));
            }
        }
        Ok(())
    }

    /// L'invariant d'ordonnancement, revalidé sur TOUTES les paires à
    /// chaque mutation du registre : si une passe écrit une cible qu'une
    /// autre lit, l'écrivaine doit précéder la lectrice dans l'ordre
    /// d'exécution (tri stable par ordre puis enregistrement). Une
    /// lecture sans écrivain la même frame reste légale (le contenu
    /// d'une frame précédente). L'invariant est DÉCLARATIF : indifférent
    /// à `enabled`.
    fn validate_ordering(&self, descriptors: &[&RenderPassDescriptor]) -> ChaosResult<()> {
        let mut schedule: Vec<usize> = (0..descriptors.len()).collect();
        schedule.sort_by_key(|&index| descriptors[index].order);
        let mut position = vec![0; descriptors.len()];
        for (rank, &index) in schedule.iter().enumerate() {
            position[index] = rank;
        }
        for (reader_index, reader) in descriptors.iter().enumerate() {
            for read in &reader.reads {
                for (writer_index, writer) in descriptors.iter().enumerate() {
                    if writer_index == reader_index {
                        continue;
                    }
                    if writer.destination == RenderDestination::Target(*read)
                        && position[writer_index] > position[reader_index]
                    {
                        let target = self
                            .lifetime
                            .render_target_info(*read)
                            .map(|info| info.label.clone())
                            .unwrap_or_else(|| String::from("render target"));
                        return Err(ChaosError::Graphics(format!(
                            "render pass '{}' writes '{target}' after pass '{}' reads it: schedule it earlier",
                            writer.label, reader.label
                        )));
                    }
                }
            }
        }
        Ok(())
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

    /// L'ANALYSE d'une liste de draws RÉSOLUS — une itération sur les
    /// soumissions GPU (jamais les objets) : classiques/instanciés/
    /// instances/triangles, et les changements d'état EXACTS que le
    /// backend encodera (le MIROIR de sa règle de déduplication
    /// `bound_pipeline`/`bound_material`, batches debug compris).
    fn analyze_draws(draws: &[FrameDraw], debug: &[FrameDebugBatch]) -> DrawAnalysis {
        let mut analysis = DrawAnalysis::default();
        let mut bound_pipeline = None;
        let mut bound_material = None;
        for draw in draws {
            let instances = draw.instances.map_or(1, |range| {
                usize::try_from(range.count).unwrap_or(usize::MAX)
            });
            if draw.instances.is_some() {
                analysis.instanced_draws += 1;
                analysis.instances += instances;
            } else {
                analysis.classic_draws += 1;
            }
            analysis.triangles += draw.element_count as usize / 3 * instances;
            if bound_pipeline != Some(draw.pipeline) {
                analysis.pipeline_switches += 1;
                bound_pipeline = Some(draw.pipeline);
            }
            if let Some(binding) = draw.binding
                && bound_material != Some(binding)
            {
                analysis.material_switches += 1;
                bound_material = Some(binding);
            }
        }
        for batch in debug {
            analysis.debug_segments += batch.vertex_count as usize / 2;
            if bound_pipeline != Some(batch.pipeline) {
                analysis.pipeline_switches += 1;
                bound_pipeline = Some(batch.pipeline);
            }
        }
        analysis
    }

    /// Clôt les diagnostics de la frame : les totaux sommés des passes,
    /// les cumulatifs (surface, budget) avancés, les coûts arrêtés, le
    /// temps GPU copié du backend — la photo stockée jusqu'au prochain
    /// `render_frame`. `outcome: None` = rien n'est parti au backend
    /// (plan vide) : aucun événement de surface n'est compté.
    #[allow(clippy::too_many_arguments)]
    fn finish_diagnostics(
        &mut self,
        submitted: usize,
        passes: Vec<PassStats>,
        shadow: Option<ShadowDiagnostics>,
        debug_segments: usize,
        resolve_ms: f32,
        backend_ms: f32,
        frame_started: Instant,
        outcome: Option<FrameOutcome>,
    ) {
        let mut totals = FrameTotals {
            submitted,
            debug_segments,
            ..FrameTotals::default()
        };
        for pass in &passes {
            totals.resolved += pass.draws;
            totals.classic_draws += pass.classic_draws;
            totals.instanced_draws += pass.instanced_draws;
            totals.instances += pass.instances;
            totals.culled += pass.culled;
            totals.triangles += pass.triangles;
            totals.pipeline_switches += pass.pipeline_switches;
            totals.material_switches += pass.material_switches;
        }
        for report in &self.report.passes {
            totals.injected += report.breakdown.injected;
            if report.outcome == PassOutcome::Executed {
                totals.passes_executed += 1;
            } else {
                totals.passes_skipped += 1;
            }
        }
        match outcome {
            Some(FrameOutcome::Rendered) => self.diagnostics.surface.presented += 1,
            Some(FrameOutcome::Skipped(reason)) => match reason {
                crate::frame::FrameSkipReason::SurfaceUnavailable => {
                    self.diagnostics.surface.skipped_unavailable += 1;
                }
                crate::frame::FrameSkipReason::SurfaceReconfigured => {
                    self.diagnostics.surface.reconfigured += 1;
                }
                crate::frame::FrameSkipReason::ZeroArea => {
                    self.diagnostics.surface.zero_area += 1;
                }
            },
            None => {}
        }
        let total_ms = frame_started.elapsed().as_secs_f32() * 1000.0;
        let over_budget = self
            .diagnostics
            .budget
            .budget_ms
            .is_some_and(|budget| total_ms > budget);
        self.diagnostics.budget.last_frame_over = over_budget;
        if over_budget {
            self.diagnostics.budget.over_budget_frames += 1;
        }
        // Les fallbacks ACTIFS : chaque permutation en échec mémoïsé est
        // un chemin dégradé assumé, les builtins vivants sont les filets.
        let degraded = self
            .sky_pipelines
            .values()
            .filter(|pipeline| pipeline.is_none())
            .count()
            + self
                .shadow_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count()
            + self
                .instanced_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count()
            + self
                .debug_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count();
        self.diagnostics.fallbacks = FallbackStats {
            degraded_permutations: degraded,
            fallback_textures: self.lifetime.fallback_texture_count(),
            fallback_samplers: self.lifetime.fallback_sampler_count(),
        };
        self.diagnostics.frame = totals;
        self.diagnostics.passes = passes;
        self.diagnostics.shadow = shadow;
        self.diagnostics.cpu = CpuCost {
            resolve_ms,
            backend_ms,
            total_ms,
        };
        self.diagnostics.gpu = self.backend.gpu_frame_time();
        self.diagnostics.resources = self.resource_stats();
    }

    /// Dérive la passe d'ombre du plan : des réglages posés ET une
    /// directionnelle qui projette — la PREMIÈRE de la collection
    /// (filtrée, tronquée : jamais un index hors du tableau GPU), la
    /// règle « les premières gagnent » de la troncature. Sans l'un ou
    /// l'autre → pas de passe : `enabled` reste à 0 côté GPU, le facteur
    /// d'ombre vaut 1 partout, rien de fatal. ZÉRO caster reste une
    /// passe : la map est effacée — jamais d'ombre fantôme d'une frame
    /// précédente.
    fn derive_shadow_pass(
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
    fn resolve_pass_draws(
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
    fn batch_shadow_casters(
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
    fn flush_retired(&mut self) {
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

#[cfg(test)]
mod tests {
    use chaos_core::Transform;
    use chaos_core::math::{Vec3, projection};

    use crate::debug::DEFAULT_DEBUG_CATEGORY;
    use crate::frame::FrameSkipReason;
    use crate::resources::{
        SamplerAddressMode, SamplerFilter, ShaderSource, TextureFormat, TextureMips,
    };
    use crate::shaders::builtin;
    use crate::shadow::ShadowVolume;
    use crate::testing::{
        Journal, create_pipeline_lines, mock_renderer, mock_renderer_with,
        mock_renderer_with_limits, render_lines, set_shadow_lines, shadow_lines,
    };

    use super::*;

    fn inline_descriptor(label: &str) -> PipelineDescriptor {
        PipelineDescriptor::new(label, ShaderSource::Wgsl(String::from("inline-code")))
    }

    fn triangle() -> Geometry {
        Geometry::triangle(
            [0.0, 0.0, 0.0],
            1.0,
            [Color::WHITE, Color::WHITE, Color::WHITE],
        )
    }

    fn quad() -> Geometry {
        Geometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, Color::WHITE)
    }

    fn cube() -> Geometry {
        Geometry::cube([0.0, 0.0, 0.0], 1.0, [Color::WHITE; 6])
    }

    fn plain_material(renderer: &mut Renderer, label: &str) -> MaterialHandle {
        renderer
            .create_material(&MaterialDescriptor::new(label, MaterialModel::VertexColor))
            .unwrap()
    }

    fn small_texture(renderer: &mut Renderer, label: &str) -> TextureHandle {
        renderer
            .create_texture(&TextureDescriptor::sampled(
                label,
                1,
                1,
                TextureFormat::R8Unorm,
                vec![7],
            ))
            .unwrap()
    }

    fn textured_material(
        renderer: &mut Renderer,
        label: &str,
        texture: TextureHandle,
        sampler: SamplerHandle,
    ) -> MaterialHandle {
        renderer
            .create_material(
                &MaterialDescriptor::new(label, MaterialModel::Unlit)
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap()
    }

    #[test]
    fn destroying_a_used_texture_is_refused_naming_the_dependents() {
        let (mut renderer, _journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "damier");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        textured_material(&mut renderer, "a", texture, sampler);
        textured_material(&mut renderer, "b", texture, sampler);
        let error = renderer.destroy_texture(texture).unwrap_err();
        assert!(error.to_string().contains("'damier'"));
        assert!(error.to_string().contains("2 material(s)"));
    }

    #[test]
    fn destroying_a_used_sampler_is_refused() {
        let (mut renderer, _journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "t");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("lecture"))
            .unwrap();
        textured_material(&mut renderer, "a", texture, sampler);
        let error = renderer.destroy_sampler(sampler).unwrap_err();
        assert!(error.to_string().contains("'lecture'"));
        assert!(error.to_string().contains("still used"));
    }

    #[test]
    fn destroying_a_mesh_owned_buffer_is_refused() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.create_mesh("quad", &quad()).unwrap();
        let owned = BufferHandle {
            index: 0,
            generation: 0,
        };
        let error = renderer.destroy_buffer(owned).unwrap_err();
        assert!(error.to_string().contains("owned by mesh 'quad'"));
        assert!(error.to_string().contains("destroy the mesh instead"));
    }

    #[test]
    fn destroying_the_material_releases_its_shares() {
        let (mut renderer, _journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "t");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        let material = textured_material(&mut renderer, "a", texture, sampler);
        assert!(renderer.destroy_texture(texture).is_err());
        renderer.destroy_material(material).unwrap();
        renderer.destroy_texture(texture).unwrap();
        renderer.destroy_sampler(sampler).unwrap();
    }

    #[test]
    fn sharing_counts_every_consumer() {
        let (mut renderer, _journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "partagee");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        let first = textured_material(&mut renderer, "a", texture, sampler);
        let second = textured_material(&mut renderer, "b", texture, sampler);
        renderer.destroy_material(first).unwrap();
        let error = renderer.destroy_texture(texture).unwrap_err();
        assert!(error.to_string().contains("1 material(s)"));
        renderer.destroy_material(second).unwrap();
        renderer.destroy_texture(texture).unwrap();
    }

    #[test]
    fn backend_release_is_deferred_to_the_end_of_the_next_frame() {
        let (mut renderer, journal) = mock_renderer();
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        assert!(
            !journal
                .entries()
                .iter()
                .any(|entry| entry.starts_with("destroy_buffer"))
        );
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        let render_position = entries
            .iter()
            .position(|entry| entry.starts_with("render"))
            .unwrap();
        let destroy_position = entries
            .iter()
            .position(|entry| entry.starts_with("destroy_buffer"))
            .unwrap();
        assert!(destroy_position > render_position);
    }

    #[test]
    fn retired_resources_drain_after_a_frame() {
        let (mut renderer, _journal) = mock_renderer();
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let texture = small_texture(&mut renderer, "t");
        renderer.destroy_mesh(mesh).unwrap();
        renderer.destroy_texture(texture).unwrap();
        assert_eq!(renderer.resource_stats().retired, 2);
        renderer.render_frame().unwrap();
        assert_eq!(renderer.resource_stats().retired, 0);
    }

    #[test]
    fn resource_stats_count_bytes_exactly() {
        let (mut renderer, _journal) = mock_renderer();
        renderer
            .create_buffer(&BufferDescriptor::vertex("v", vec![0; 24]))
            .unwrap();
        renderer
            .create_texture(&TextureDescriptor::sampled(
                "t",
                2,
                2,
                TextureFormat::Rgba8UnormSrgb,
                vec![0; 16],
            ))
            .unwrap();
        let stats = renderer.resource_stats();
        assert_eq!(stats.buffers.alive, 1);
        assert_eq!(stats.buffers.bytes, 24);
        assert_eq!(stats.textures.alive, 1);
        assert_eq!(stats.textures.bytes, 16);
        assert_eq!(stats.estimated_bytes, 40);
    }

    #[test]
    fn stats_return_to_baseline_after_destruction() {
        let (mut renderer, _journal) = mock_renderer();
        // Les fallbacks (blanche, normale plate) sont PROTÉGÉS et
        // persistent : la baseline se prend après leur résolution.
        renderer.builtin_texture(BuiltinTexture::White).unwrap();
        renderer
            .builtin_texture(BuiltinTexture::FlatNormal)
            .unwrap();
        let baseline = renderer.resource_stats();
        let texture = small_texture(&mut renderer, "t");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        let material = textured_material(&mut renderer, "m", texture, sampler);
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.destroy_material(material).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        renderer.destroy_texture(texture).unwrap();
        renderer.destroy_sampler(sampler).unwrap();
        renderer.render_frame().unwrap();
        let stats = renderer.resource_stats();
        assert_eq!(stats.buffers, baseline.buffers);
        assert_eq!(stats.textures, baseline.textures);
        assert_eq!(stats.samplers, baseline.samplers);
        assert_eq!(stats.meshes, baseline.meshes);
        assert_eq!(stats.materials, baseline.materials);
        assert_eq!(stats.retired, 0);
        assert_eq!(stats.estimated_bytes, baseline.estimated_bytes);
    }

    #[test]
    fn double_destroy_stays_an_explicit_stale_error() {
        let (mut renderer, _journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "t");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        let buffer = renderer
            .create_buffer(&BufferDescriptor::vertex("v", vec![1]))
            .unwrap();
        renderer.destroy_texture(texture).unwrap();
        renderer.destroy_sampler(sampler).unwrap();
        renderer.destroy_buffer(buffer).unwrap();
        assert!(
            renderer
                .destroy_texture(texture)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            renderer
                .destroy_sampler(sampler)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            renderer
                .destroy_buffer(buffer)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
    }

    #[test]
    fn pipelines_are_counted_permanent() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.create_pipeline(&inline_descriptor("a")).unwrap();
        renderer.create_pipeline(&inline_descriptor("b")).unwrap();
        assert_eq!(renderer.resource_stats().pipelines, 2);
    }

    #[test]
    fn intensive_churn_never_leaks_nor_resolves_stale() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.builtin_texture(BuiltinTexture::White).unwrap();
        renderer
            .builtin_texture(BuiltinTexture::FlatNormal)
            .unwrap();
        let baseline = renderer.resource_stats();
        let mut past_materials = Vec::new();
        let mut past_meshes = Vec::new();
        for cycle in 0..100 {
            let texture = small_texture(&mut renderer, &format!("t{cycle}"));
            let sampler = renderer
                .create_sampler(&SamplerDescriptor::new(format!("s{cycle}")))
                .unwrap();
            let first = textured_material(&mut renderer, &format!("a{cycle}"), texture, sampler);
            let second = textured_material(&mut renderer, &format!("b{cycle}"), texture, sampler);
            let mesh = renderer
                .create_mesh(&format!("m{cycle}"), &triangle())
                .unwrap();
            renderer.queue_draw(DrawCommand {
                mesh,
                material: first,
                transform: Transform::IDENTITY,
            });
            renderer.queue_draw(DrawCommand {
                mesh,
                material: second,
                transform: Transform::IDENTITY,
            });
            renderer.render_frame().unwrap();
            assert!(renderer.destroy_texture(texture).is_err());
            renderer.destroy_material(first).unwrap();
            renderer.destroy_material(second).unwrap();
            renderer.destroy_mesh(mesh).unwrap();
            renderer.destroy_texture(texture).unwrap();
            renderer.destroy_sampler(sampler).unwrap();
            renderer.clear_draws();
            renderer.render_frame().unwrap();
            past_materials.push(first);
            past_meshes.push(mesh);
        }
        let stats = renderer.resource_stats();
        assert_eq!(stats.buffers, baseline.buffers);
        assert_eq!(stats.textures, baseline.textures);
        assert_eq!(stats.samplers, baseline.samplers);
        assert_eq!(stats.meshes, baseline.meshes);
        assert_eq!(stats.materials, baseline.materials);
        assert_eq!(stats.retired, 0);
        for material in past_materials {
            assert!(renderer.destroy_material(material).is_err());
        }
        for mesh in past_meshes {
            assert!(renderer.destroy_mesh(mesh).is_err());
        }
    }

    #[test]
    fn provided_mips_reach_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        let mut pixels = vec![0u8; 4 * 4 * 4];
        pixels.extend_from_slice(&[0u8; 2 * 2 * 4]);
        pixels.extend_from_slice(&[0u8; 4]);
        renderer
            .create_texture(
                &TextureDescriptor::sampled("mippee", 4, 4, TextureFormat::Rgba8Unorm, pixels)
                    .with_mips(TextureMips::Provided(3)),
            )
            .unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_texture mippee 4x4 format=Rgba8Unorm usage=Sampled bytes=84 levels=3"]
        );
    }

    #[test]
    fn generate_is_resolved_before_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_texture(
                &TextureDescriptor::sampled("auto", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                    .with_mips(TextureMips::Generate),
            )
            .unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_texture auto 2x2 format=Rgba8Unorm usage=Sampled bytes=20 levels=2"]
        );
    }

    #[test]
    fn a_cubemap_reaches_the_backend_with_its_layers() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_texture(&TextureDescriptor::cube(
                "env",
                2,
                TextureFormat::Rgba16Float,
                vec![0; 2 * 2 * 8 * 6],
            ))
            .unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_texture env 2x2 format=Rgba16Float usage=Sampled bytes=192 kind=Cube"]
        );
    }

    #[test]
    fn update_texture_is_validated_then_forwarded() {
        let (mut renderer, journal) = mock_renderer();
        let texture = small_texture(&mut renderer, "dynamique");
        renderer.update_texture(texture, &[42]).unwrap();
        assert!(
            journal
                .entries()
                .contains(&String::from("update_texture index=0 bytes=1"))
        );

        let wrong_len = renderer.update_texture(texture, &[1, 2, 3]).unwrap_err();
        assert!(wrong_len.to_string().contains("expects 1 bytes"));

        renderer.destroy_texture(texture).unwrap();
        let stale = renderer.update_texture(texture, &[42]).unwrap_err();
        assert!(stale.to_string().contains("stale"));
    }

    #[test]
    fn mipmapped_and_cube_textures_refuse_updates() {
        let (mut renderer, _journal) = mock_renderer();
        let mipped = renderer
            .create_texture(
                &TextureDescriptor::sampled("mippee", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                    .with_mips(TextureMips::Generate),
            )
            .unwrap();
        let error = renderer.update_texture(mipped, &[0; 16]).unwrap_err();
        assert!(error.to_string().contains("single-level 2D"));

        let cube = renderer
            .create_texture(&TextureDescriptor::cube(
                "env",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 24],
            ))
            .unwrap();
        let error = renderer.update_texture(cube, &[0; 24]).unwrap_err();
        assert!(error.to_string().contains("single-level 2D"));
    }

    #[test]
    fn a_material_refuses_a_cubemap_texture() {
        let (mut renderer, _journal) = mock_renderer();
        let cube = renderer
            .create_texture(&TextureDescriptor::cube(
                "env",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 24],
            ))
            .unwrap();
        let error = renderer
            .create_material(&MaterialDescriptor::new("m", MaterialModel::Unlit).with_texture(cube))
            .unwrap_err();
        assert!(error.to_string().contains("cubemap"));
        assert!(error.to_string().contains("environment pass"));
    }

    #[test]
    fn builtin_textures_are_lazy_shared_and_protected() {
        let (mut renderer, journal) = mock_renderer();
        let white = renderer.builtin_texture(BuiltinTexture::White).unwrap();
        let white_again = renderer.builtin_texture(BuiltinTexture::White).unwrap();
        let black = renderer.builtin_texture(BuiltinTexture::Black).unwrap();
        let normal = renderer
            .builtin_texture(BuiltinTexture::FlatNormal)
            .unwrap();
        assert_eq!(white, white_again);
        assert_ne!(white, black);
        assert_ne!(black, normal);
        assert_eq!(
            journal
                .entries()
                .iter()
                .filter(|entry| entry.starts_with("create_texture chaos."))
                .count(),
            3
        );
        for handle in [white, black, normal] {
            let error = renderer.destroy_texture(handle).unwrap_err();
            assert!(error.to_string().contains("builtin fallback"));
            let error = renderer.update_texture(handle, &[0; 4]).unwrap_err();
            assert!(error.to_string().contains("builtin fallback"));
        }
    }

    #[test]
    fn stats_count_the_full_mip_chain_bytes() {
        let (mut renderer, _journal) = mock_renderer();
        renderer
            .create_texture(
                &TextureDescriptor::sampled("mippee", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                    .with_mips(TextureMips::Generate),
            )
            .unwrap();
        assert_eq!(renderer.resource_stats().textures.bytes, 20);
    }

    #[test]
    fn an_anisotropic_sampler_is_validated_before_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        let error = renderer
            .create_sampler(&SamplerDescriptor::new("bad").with_anisotropy(4))
            .unwrap_err();
        assert!(error.to_string().contains("Linear filtering everywhere"));
        assert!(journal.entries().is_empty());

        renderer
            .create_sampler(
                &SamplerDescriptor::new("aniso")
                    .with_mip_filter(SamplerFilter::Linear)
                    .with_anisotropy(8),
            )
            .unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_sampler aniso filter=Linear address=Repeat mips=Linear aniso=8"]
        );
    }

    fn small_target(renderer: &mut Renderer, label: &str) -> RenderTargetHandle {
        renderer
            .create_render_target(&RenderTargetDescriptor::new(
                label,
                4,
                4,
                TextureFormat::Rgba8UnormSrgb,
            ))
            .unwrap()
    }

    #[test]
    fn a_render_target_reaches_the_backend_and_the_stats() {
        let (mut renderer, journal) = mock_renderer();
        small_target(&mut renderer, "viewport");
        assert_eq!(
            journal.entries(),
            vec!["create_render_target viewport 4x4 format=Rgba8UnormSrgb"]
        );
        let stats = renderer.resource_stats();
        assert_eq!(stats.render_targets.alive, 1);
        assert_eq!(stats.render_targets.bytes, 4 * 4 * 4);
        assert_eq!(stats.textures.alive, 1);
        assert_eq!(stats.textures.bytes, 4 * 4 * 4);
    }

    #[test]
    fn the_target_color_feeds_a_material() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let color = renderer.render_target_color(target).unwrap();
        renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
            )
            .unwrap();
        assert!(
            journal
                .entries()
                .iter()
                .any(|entry| entry.starts_with("create_material_binding screen texture=0"))
        );
    }

    #[test]
    fn render_to_target_orders_resolves_and_targets() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        let main_draws_before = renderer.draw_count();
        renderer
            .render_to_target(
                target,
                Color::BLACK,
                Mat4::IDENTITY,
                &[DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                }],
            )
            .unwrap();
        assert_eq!(renderer.draw_count(), main_draws_before);
        let entries = journal.entries();
        let target_render = entries
            .iter()
            .find(|entry| entry.contains("dest=target0"))
            .unwrap();
        // La permutation surface (pipeline 0, l'eager) et la permutation
        // cible (pipeline 1) sont deux pipelines distincts.
        assert!(target_render.contains("draws=[(1,"));
    }

    #[test]
    fn stale_target_operations_are_explicit_errors() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer.destroy_render_target(target).unwrap();
        assert!(
            renderer
                .render_target_color(target)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            renderer
                .render_target_size(target)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            renderer
                .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            renderer
                .destroy_render_target(target)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
    }

    #[test]
    fn stale_draws_are_dropped_from_target_passes_too() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let command = DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        };
        renderer.destroy_mesh(mesh).unwrap();
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command])
            .unwrap();
        assert!(
            journal
                .entries()
                .iter()
                .any(|entry| entry.contains("draws=[] dest=target0"))
        );
    }

    #[test]
    fn resize_rotates_the_handles() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let old_color = renderer.render_target_color(target).unwrap();
        let resized = renderer.resize_render_target(target, 8, 8).unwrap();
        assert_ne!(target, resized);
        assert_eq!(renderer.render_target_size(resized).unwrap(), (8, 8));
        assert!(renderer.render_target_color(target).is_err());
        let new_color = renderer.render_target_color(resized).unwrap();
        assert_ne!(old_color, new_color);
        assert!(renderer.resource_stats().retired > 0);
        renderer.render_frame().unwrap();
        assert_eq!(renderer.resource_stats().retired, 0);
    }

    #[test]
    fn destroying_a_target_is_refused_while_its_color_is_shared() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let color = renderer.render_target_color(target).unwrap();
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
            )
            .unwrap();
        let error = renderer.destroy_render_target(target).unwrap_err();
        assert!(error.to_string().contains("still used by 1 material(s)"));
        renderer.destroy_material(material).unwrap();
        renderer.destroy_render_target(target).unwrap();
    }

    #[test]
    fn target_stats_return_to_baseline() {
        let (mut renderer, _journal) = mock_renderer();
        let baseline = renderer.resource_stats();
        let target = small_target(&mut renderer, "viewport");
        renderer.destroy_render_target(target).unwrap();
        renderer.render_frame().unwrap();
        let stats = renderer.resource_stats();
        assert_eq!(stats.render_targets, baseline.render_targets);
        assert_eq!(stats.textures, baseline.textures);
        assert_eq!(stats.retired, 0);
    }

    #[test]
    fn a_pipeline_color_target_reaches_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_pipeline(
                &inline_descriptor("offscreen").with_color_target(TextureFormat::Rgba16Float),
            )
            .unwrap();
        assert!(
            journal
                .entries()
                .iter()
                .any(|entry| entry.contains("target=Rgba16Float"))
        );
    }

    #[test]
    fn offscreen_then_display_is_the_full_checkpoint_flow() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        // UN seul material de scène : la permutation offscreen se résout
        // seule au moment du rendu vers la cible — plus de duplication.
        let scene_material = plain_material(&mut renderer, "scene");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let screen_mesh = renderer
            .create_textured_mesh("screen", &screen_quad)
            .unwrap();
        renderer
            .render_to_target(
                target,
                Color::BLACK,
                Mat4::IDENTITY,
                &[DrawCommand {
                    mesh,
                    material: scene_material,
                    transform: Transform::IDENTITY,
                }],
            )
            .unwrap();

        let color = renderer.render_target_color(target).unwrap();
        let screen_material = renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
            )
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: screen_mesh,
            material: screen_material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();

        renderer.destroy_material(screen_material).unwrap();
        let resized = renderer.resize_render_target(target, 8, 8).unwrap();
        let new_color = renderer.render_target_color(resized).unwrap();
        let screen_material = renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(new_color),
            )
            .unwrap();
        renderer
            .render_to_target(
                resized,
                Color::BLACK,
                Mat4::IDENTITY,
                &[DrawCommand {
                    mesh,
                    material: scene_material,
                    transform: Transform::IDENTITY,
                }],
            )
            .unwrap();
        renderer.clear_draws();
        renderer.queue_draw(DrawCommand {
            mesh: screen_mesh,
            material: screen_material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();

        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("dest=target"))
                .count(),
            2
        );
        assert!(entries.iter().any(|entry| entry.contains("dest=target1")));
        // Les deux passes cible dessinent : la permutation offscreen du
        // material de scène a bien été résolue (jamais un draw vide).
        assert!(
            entries
                .iter()
                .filter(|entry| entry.contains("dest=target"))
                .all(|entry| entry.contains("draws=[("))
        );
    }

    #[test]
    fn frame_plan_carries_current_clear_color() {
        let (mut renderer, journal) = mock_renderer();
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"]
        );
    }

    #[test]
    fn set_clear_color_changes_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_clear_color(Color::rgb(1.0, 0.5, 0.25));
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=1 g=0.5 b=0.25 a=1 vp=[0, 0, 0] draws=[]"]
        );
        assert_eq!(renderer.clear_color(), Color::rgb(1.0, 0.5, 0.25));
    }

    #[test]
    fn backend_outcome_is_propagated() {
        let skipped = FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable);
        let (mut renderer, _journal) = mock_renderer_with(skipped);
        assert_eq!(renderer.render_frame().unwrap(), skipped);
    }

    #[test]
    fn resize_is_forwarded_to_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer.resize(1920, 1080);
        assert_eq!(journal.entries(), vec!["resize 1920x1080"]);
        assert_eq!(renderer.surface_size(), (1920, 1080));
        renderer.resize(0, 0);
        assert_eq!(renderer.surface_size(), (1920, 1080));
    }

    #[test]
    fn description_delegates_to_backend() {
        let (renderer, _journal) = mock_renderer();
        assert_eq!(renderer.description(), "mock backend");
    }

    #[test]
    fn create_pipeline_returns_increasing_handles() {
        let (mut renderer, _journal) = mock_renderer();
        let first = renderer.create_pipeline(&inline_descriptor("a")).unwrap();
        let second = renderer.create_pipeline(&inline_descriptor("b")).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn inline_shader_reaches_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer.create_pipeline(&inline_descriptor("a")).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_pipeline a code=inline-code"]
        );
    }

    #[test]
    fn named_shader_resolves_through_the_library() {
        let (mut renderer, journal) = mock_renderer();
        renderer.shaders_mut().register(
            "game.custom",
            ShaderSource::Wgsl(String::from("custom-code")),
        );
        let descriptor = PipelineDescriptor::new("t", "game.custom");
        renderer.create_pipeline(&descriptor).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_pipeline t code=custom-code"]
        );
    }

    #[test]
    fn unknown_named_shader_is_a_comprehensible_error() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor = PipelineDescriptor::new("t", "missing.shader");
        let error = renderer.create_pipeline(&descriptor).unwrap_err();
        assert!(error.to_string().contains("missing.shader"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn builtin_vertex_color_is_available() {
        let (renderer, _journal) = mock_renderer();
        assert!(renderer.shaders().contains(builtin::VERTEX_COLOR));
    }

    #[test]
    fn create_buffer_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_buffer(&BufferDescriptor::vertex("tri", vec![0, 1, 2, 3]))
            .unwrap();
        let second = renderer
            .create_buffer(&BufferDescriptor::index("idx", vec![0, 1]))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer tri kind=Vertex bytes=4",
                "create_buffer idx kind=Index bytes=2"
            ]
        );
    }

    #[test]
    fn destroy_buffer_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_buffer(&BufferDescriptor::vertex("tri", Vec::new()))
            .unwrap();
        renderer.destroy_buffer(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_buffer tri kind=Vertex bytes=0"]
        );
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries().last().map(String::as_str),
            Some("destroy_buffer index=0")
        );
    }

    #[test]
    fn create_texture_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_texture(&TextureDescriptor::sampled(
                "albedo",
                2,
                2,
                TextureFormat::Rgba8UnormSrgb,
                vec![255; 16],
            ))
            .unwrap();
        let second = renderer
            .create_texture(&TextureDescriptor::render_target(
                "offscreen",
                4,
                4,
                TextureFormat::Rgba8Unorm,
            ))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_texture albedo 2x2 format=Rgba8UnormSrgb usage=Sampled bytes=16",
                "create_texture offscreen 4x4 format=Rgba8Unorm usage=RenderTarget bytes=0"
            ]
        );
    }

    #[test]
    fn destroy_texture_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_texture(&TextureDescriptor::sampled(
                "mask",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![128],
            ))
            .unwrap();
        renderer.destroy_texture(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_texture mask 1x1 format=R8Unorm usage=Sampled bytes=1"]
        );
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries().last().map(String::as_str),
            Some("destroy_texture index=0")
        );
    }

    #[test]
    fn texture_with_wrong_pixel_size_is_rejected_before_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        let error = renderer
            .create_texture(&TextureDescriptor::sampled(
                "bad",
                2,
                2,
                TextureFormat::Rgba8Unorm,
                vec![0; 3],
            ))
            .unwrap_err();
        assert!(error.to_string().contains("16 bytes"));
        assert!(error.to_string().contains("got 3"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn render_target_with_initial_pixels_is_rejected() {
        let (mut renderer, journal) = mock_renderer();
        let mut descriptor =
            TextureDescriptor::render_target("rt", 2, 2, TextureFormat::Rgba8Unorm);
        descriptor.pixels = vec![0; 16];
        let error = renderer.create_texture(&descriptor).unwrap_err();
        assert!(error.to_string().contains("render target"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn zero_sized_texture_is_rejected() {
        let (mut renderer, journal) = mock_renderer();
        let error = renderer
            .create_texture(&TextureDescriptor::sampled(
                "empty",
                0,
                4,
                TextureFormat::R8Unorm,
                Vec::new(),
            ))
            .unwrap_err();
        assert!(error.to_string().contains("zero dimensions"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn create_sampler_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_sampler(&SamplerDescriptor::new("linear"))
            .unwrap();
        let second = renderer
            .create_sampler(
                &SamplerDescriptor::new("pixel")
                    .with_filter(SamplerFilter::Nearest)
                    .with_address_mode(SamplerAddressMode::ClampToEdge),
            )
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_sampler linear filter=Linear address=Repeat",
                "create_sampler pixel filter=Nearest address=ClampToEdge"
            ]
        );
    }

    #[test]
    fn destroy_sampler_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        renderer.destroy_sampler(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_sampler s filter=Linear address=Repeat"]
        );
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries().last().map(String::as_str),
            Some("destroy_sampler index=0")
        );
    }

    fn texture_and_sampler(renderer: &mut Renderer) -> (TextureHandle, SamplerHandle) {
        let texture = renderer
            .create_texture(&TextureDescriptor::sampled(
                "albedo",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        (texture, sampler)
    }

    #[test]
    fn get_or_create_texture_deduplicates_by_label() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor =
            TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
        let first = renderer.get_or_create_texture(&descriptor).unwrap();
        let second = renderer.get_or_create_texture(&descriptor).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            journal.entries(),
            vec!["create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"]
        );
    }

    #[test]
    fn get_or_create_texture_recreates_after_destroy() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor =
            TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
        let first = renderer.get_or_create_texture(&descriptor).unwrap();
        renderer.destroy_texture(first).unwrap();
        let second = renderer.get_or_create_texture(&descriptor).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1",
                "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"
            ]
        );
        renderer.render_frame().unwrap();
        assert!(
            journal
                .entries()
                .contains(&String::from("destroy_texture index=0"))
        );
    }

    #[test]
    fn distinct_labels_create_distinct_textures() {
        let (mut renderer, _journal) = mock_renderer();
        let first = renderer
            .get_or_create_texture(&TextureDescriptor::sampled(
                "a",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        let second = renderer
            .get_or_create_texture(&TextureDescriptor::sampled(
                "b",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn builtin_fallbacks_are_protected() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_material(&MaterialDescriptor::new("a", MaterialModel::Unlit))
            .unwrap();
        let fallback_texture = TextureHandle {
            index: 0,
            generation: 0,
        };
        let texture_error = renderer.destroy_texture(fallback_texture).unwrap_err();
        assert!(texture_error.to_string().contains("builtin fallback"));
        let fallback_sampler = SamplerHandle {
            index: 0,
            generation: 0,
        };
        let sampler_error = renderer.destroy_sampler(fallback_sampler).unwrap_err();
        assert!(sampler_error.to_string().contains("builtin fallback"));
        renderer
            .create_material(&MaterialDescriptor::new("b", MaterialModel::Unlit))
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("create_texture chaos.white"))
                .count(),
            1
        );
    }

    #[test]
    fn create_material_uses_builtin_fallbacks_once() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_material(&MaterialDescriptor::new("a", MaterialModel::Unlit))
            .unwrap();
        renderer
            .create_material(&MaterialDescriptor::new("b", MaterialModel::Unlit))
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("chaos.white"))
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("chaos.default_sampler"))
                .count(),
            1
        );
        // Les slots PBR sont TOUJOURS remplis : la blanche (idx 0)
        // partagée par base/mr/ao/émissif, la normale plate (idx 1).
        assert_eq!(
            entries[entries.len() - 1],
            "create_material_binding b texture=0 sampler=0 color=(1, 1, 1, 1) mr=0 normal=1 ao=0 em=0"
        );
    }

    #[test]
    fn create_material_forwards_texture_sampler_and_color() {
        let (mut renderer, journal) = mock_renderer();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Unlit)
                    .with_base_color(Color::rgb(0.5, 0.25, 1.0))
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "create_material_binding m texture=0 sampler=0 color=(0.5, 0.25, 1, 1) mr=1 normal=2 ao=1 em=1"
        );
    }

    #[test]
    fn destroy_material_destroys_its_binding() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        renderer.destroy_material(material).unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "destroy_material_binding index=0"
        );
        let error = renderer.destroy_material(material).unwrap_err();
        assert!(error.to_string().contains("stale"));
    }

    #[test]
    fn the_renderer_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Renderer>();
    }

    #[test]
    fn draw_count_reports_the_submitted_frame() {
        let (mut renderer, _journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        assert_eq!(renderer.draw_count(), 0);
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        assert_eq!(renderer.draw_count(), 2);
        renderer.clear_draws();
        assert_eq!(renderer.draw_count(), 0);
    }

    #[test]
    fn stale_material_draw_is_dropped_from_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.destroy_material(material).unwrap();
        renderer.render_frame().unwrap();
        assert!(journal.entries().contains(&String::from(
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        )));
    }

    #[test]
    fn create_mesh_uploads_the_right_buffers() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer.create_mesh("tri", &triangle()).unwrap();
        let second = renderer.create_mesh("quad", &quad()).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer tri kind=Vertex bytes=72",
                "create_buffer quad kind=Vertex bytes=96",
                "create_buffer quad.indices kind=Index bytes=12"
            ]
        );
    }

    #[test]
    fn create_textured_mesh_uploads_the_uv_vertices() {
        let (mut renderer, journal) = mock_renderer();
        let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 4.0);
        renderer.create_textured_mesh("floor", &quad).unwrap();
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer floor kind=Vertex bytes=80",
                "create_buffer floor.indices kind=Index bytes=12"
            ]
        );
    }

    #[test]
    fn mesh_draws_resolve_into_the_plan_then_reset() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let tri = renderer.create_mesh("tri", &triangle()).unwrap();
        let quad = renderer.create_mesh("quad", &quad()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: tri,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh: quad,
            material,
            transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        renderer.clear_draws();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        let full_plan = "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[(0, Some(0), None, 3, b=Some(0), m=[0, 0, 0]), (0, Some(1), Some(2), 6, b=Some(0), m=[5, 0, 0])]";
        assert_eq!(entries[entries.len() - 3], full_plan);
        assert_eq!(entries[entries.len() - 2], full_plan);
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
    }

    #[test]
    fn interleaved_materials_are_grouped_in_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let first = plain_material(&mut renderer, "a");
        let second = plain_material(&mut renderer, "b");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let submissions = [(second, 1.0), (first, 2.0), (second, 3.0), (first, 4.0)];
        for (material, x) in submissions {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        // Les deux materials partagent le MÊME modèle et le même état :
        // les runs de chacun (mesh partagé) FUSIONNENT en un draw
        // instancié — la permutation instanciée (pipeline 1, lazy à la
        // première frame) est DÉDUPLIQUÉE entre les deux, seuls les
        // bindings diffèrent ; la matrice du draw est celle de la
        // PREMIÈRE instance, l'ordre de soumission tient dans le run.
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
             (1, Some(0), None, 3, b=Some(0), m=[2, 0, 0], inst=2), \
             (1, Some(0), None, 3, b=Some(1), m=[1, 0, 0], inst=2)]"
        );
        assert_eq!(
            create_pipeline_lines(&journal)
                .iter()
                .filter(|line| line.contains(" instanced"))
                .count(),
            1
        );
    }

    #[test]
    fn shared_mesh_draws_with_distinct_transforms() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        for x in [-2.0, 0.0, 2.0] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        // Le motif « un mesh, N draws » est devenu LA forme instanciée :
        // UN draw, trois instances, la matrice de la première en tête.
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
             (1, Some(0), Some(1), 36, b=Some(0), m=[-2, 0, 0], inst=3)]"
        );
    }

    #[test]
    fn many_draws_reach_the_plan_in_submission_order() {
        // Le chemin CLASSIQUE : seize meshes distincts (aucun run à
        // fusionner) — chaque draw garde son slot, l'ordre déterministe
        // du tri (material, mesh) suit l'ordre de création.
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        for index in 0u8..16 {
            let mesh = renderer
                .create_mesh(&format!("cube.{index}"), &cube())
                .unwrap();
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(f32::from(index), 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        let plan = entries[entries.len() - 1].clone();
        assert_eq!(plan.matches("m=[").count(), 16);
        let positions: Vec<usize> = (0u8..16)
            .map(|index| {
                plan.find(&format!("m=[{}, 0, 0]", f32::from(index)))
                    .unwrap()
            })
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn meshes_carry_their_local_bounds() {
        let (mut renderer, _journal) = mock_renderer();
        // Le cube unitaire à l'origine : bounds exacts ±0.5.
        let cube = renderer.create_mesh("cube", &cube()).unwrap();
        let bounds = renderer.mesh_bounds(cube).unwrap().unwrap();
        assert_eq!(bounds.min, Vec3::splat(-0.5));
        assert_eq!(bounds.max, Vec3::splat(0.5));
        // Une géométrie VIDE n'a pas de bounds — jamais cullée.
        let empty = renderer
            .create_mesh(
                "empty",
                &Geometry {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                },
            )
            .unwrap();
        assert!(renderer.mesh_bounds(empty).unwrap().is_none());
        // Une position non finie REFUSE les bounds (warn) — jamais cullé.
        let broken = renderer
            .create_mesh(
                "broken",
                &Geometry {
                    vertices: vec![ColorVertex {
                        position: [f32::NAN, 0.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    }],
                    indices: Vec::new(),
                },
            )
            .unwrap();
        assert!(renderer.mesh_bounds(broken).unwrap().is_none());
        // L'inspection refuse un handle périmé.
        renderer.destroy_mesh(broken).unwrap();
        assert!(renderer.mesh_bounds(broken).is_err());
    }

    #[test]
    fn destroy_mesh_destroys_its_buffers() {
        let (mut renderer, journal) = mock_renderer();
        let mesh = renderer.create_mesh("quad", &quad()).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert!(entries.contains(&String::from("destroy_buffer index=0")));
        assert!(entries.contains(&String::from("destroy_buffer index=1")));
        let error = renderer.destroy_mesh(mesh).unwrap_err();
        assert!(error.to_string().contains("stale"));
    }

    #[test]
    fn stale_mesh_draw_is_dropped_from_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.destroy_mesh(mesh).unwrap();
        renderer.render_frame().unwrap();
        assert!(journal.entries().contains(&String::from(
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        )));
    }

    #[test]
    fn view_projection_travels_in_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_view_projection(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=0 g=0 b=0 a=1 vp=[1, 2, 3] draws=[]"]
        );
    }

    fn surface_pass(label: &str) -> RenderPassDescriptor {
        RenderPassDescriptor::new(label, RenderDestination::Surface)
    }

    #[test]
    fn the_order_drives_the_schedule_not_the_registration() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .add_pass(&surface_pass("after").with_order(5))
            .unwrap();
        renderer
            .add_pass(&surface_pass("before").with_order(-5))
            .unwrap();
        renderer
            .add_pass(&surface_pass("tied").with_order(5))
            .unwrap();
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        // Deux frames — le même ordre exact : before, main, after, tied
        // (l'égalité d'ordre départagée par l'enregistrement).
        assert_eq!(lines.len(), 8);
        for frame in lines.chunks(4) {
            assert!(frame[0].ends_with(" pass=before"));
            assert!(!frame[1].contains(" pass="));
            assert!(frame[2].ends_with(" pass=after"));
            assert!(frame[3].ends_with(" pass=tied"));
        }
    }

    #[test]
    fn pass_labels_are_validated() {
        let (mut renderer, _journal) = mock_renderer();
        let empty = renderer.add_pass(&surface_pass("")).unwrap_err();
        assert!(empty.to_string().contains("cannot be empty"));
        let reserved = renderer
            .add_pass(&surface_pass("chaos.shadow"))
            .unwrap_err();
        assert!(reserved.to_string().contains("reserved for engine passes"));
        renderer.add_pass(&surface_pass("overlay")).unwrap();
        let duplicate = renderer.add_pass(&surface_pass("overlay")).unwrap_err();
        assert!(duplicate.to_string().contains("'overlay' already exists"));
    }

    #[test]
    fn invalid_dependencies_are_refused_by_name() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");

        let feedback = renderer
            .add_pass(
                &RenderPassDescriptor::new("loop", RenderDestination::Target(target))
                    .with_reads(&[target]),
            )
            .unwrap_err();
        assert!(feedback.to_string().contains("feedback loop"));

        // Lectrice à l'ordre -1, écrivaine à l'ordre 0 : l'écrivaine
        // arriverait APRÈS la lecture — refusée en nommant tout le monde.
        renderer
            .add_pass(&surface_pass("reader").with_reads(&[target]).with_order(-1))
            .unwrap();
        let writer = renderer
            .add_pass(&RenderPassDescriptor::new(
                "writer",
                RenderDestination::Target(target),
            ))
            .unwrap_err();
        let message = writer.to_string();
        assert!(message.contains("'writer' writes 'viewport' after pass 'reader' reads it"));
        assert!(message.contains("schedule it earlier"));

        // La même écrivaine AVANT la lectrice est la forme légale.
        renderer
            .add_pass(
                &RenderPassDescriptor::new("writer", RenderDestination::Target(target))
                    .with_order(-2),
            )
            .unwrap();
    }

    #[test]
    fn a_read_without_a_writer_is_legal() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .add_pass(&surface_pass("reader").with_reads(&[target]))
            .unwrap();
        renderer.render_frame().unwrap();
        // Personne n'écrit la cible cette frame : contenu d'une frame
        // précédente, la passe s'exécute quand même.
        assert_eq!(render_lines(&journal).len(), 2);
    }

    #[test]
    fn update_pass_revalidates_the_whole_schedule() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let writer = renderer
            .add_pass(
                &RenderPassDescriptor::new("writer", RenderDestination::Target(target))
                    .with_order(-2),
            )
            .unwrap();
        renderer
            .add_pass(&surface_pass("reader").with_reads(&[target]).with_order(-1))
            .unwrap();

        // Repousser l'écrivaine APRÈS la lectrice casse l'invariant
        // entre deux passes que l'update ne touche pas directement.
        let pushed =
            RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(3);
        let refused = renderer.update_pass(writer, &pushed).unwrap_err();
        assert!(refused.to_string().contains("schedule it earlier"));

        // Refus = état intact : l'ordre d'origine tient toujours.
        let kept =
            RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(-3);
        renderer.update_pass(writer, &kept).unwrap();
    }

    #[test]
    fn the_main_pass_destination_and_label_are_protected() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let main = renderer.main_pass();

        let moved = renderer
            .update_pass(
                main,
                &RenderPassDescriptor::new("chaos.main", RenderDestination::Target(target)),
            )
            .unwrap_err();
        assert!(moved.to_string().contains("destination cannot change"));

        let renamed = renderer
            .update_pass(main, &surface_pass("scene"))
            .unwrap_err();
        assert!(renamed.to_string().contains("label cannot change"));

        // load / caméra / ordre restent libres sur main.
        renderer
            .update_pass(
                main,
                &RenderPassDescriptor::new("chaos.main", RenderDestination::Surface)
                    .with_load(PassLoad::Keep)
                    .with_order(10),
            )
            .unwrap();
        renderer.render_frame().unwrap();
        assert!(render_lines(&journal)[0].ends_with(" load=keep"));
    }

    #[test]
    fn a_disabled_pass_is_skipped_cleanly() {
        let (mut renderer, journal) = mock_renderer();
        let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let command = DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        };

        renderer.set_pass_enabled(overlay, false).unwrap();
        renderer.queue_draw_to(overlay, command).unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(render_lines(&journal).len(), 1);
        let report = renderer.frame_report();
        assert_eq!(report.passes.len(), 2);
        assert_eq!(report.passes[1].label, "overlay");
        assert_eq!(report.passes[1].outcome, PassOutcome::Disabled);

        renderer.set_pass_enabled(overlay, true).unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(render_lines(&journal).len(), 3);
    }

    #[test]
    fn each_pass_owns_its_queue_and_the_count_sums_them() {
        let (mut renderer, journal) = mock_renderer();
        let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let command = DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        };

        renderer.queue_draw(command);
        renderer.queue_draw_to(overlay, command).unwrap();
        renderer.queue_draw_to(overlay, command).unwrap();
        assert_eq!(renderer.draw_count(), 3);

        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        assert_eq!(lines[0].matches("m=[").count(), 1);
        // Les deux draws identiques de l'overlay FUSIONNENT — un draw
        // instancié, deux instances : le compte logique ne ment pas.
        assert_eq!(lines[1].matches("m=[").count(), 1);
        assert!(lines[1].contains("inst=2"));
        assert_eq!(renderer.frame_report().passes[1].draws, 2);
        assert_eq!(renderer.frame_report().passes[1].draw_calls, 1);

        renderer.clear_draws();
        assert_eq!(renderer.draw_count(), 0);

        let unknown = renderer.queue_draw_to(PassHandle(42), command).unwrap_err();
        assert!(unknown.to_string().contains("unknown"));
    }

    #[test]
    fn each_pass_travels_with_its_own_camera() {
        let (mut renderer, journal) = mock_renderer();
        let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
        renderer.set_view_projection(Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)));
        renderer
            .set_pass_camera(overlay, Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)))
            .unwrap();
        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        assert!(lines[0].contains("vp=[1, 0, 0]"));
        assert!(lines[1].contains("vp=[0, 2, 0]"));
    }

    #[test]
    fn a_stale_destination_disables_the_pass_until_updated() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();

        let fresh = renderer.resize_render_target(target, 8, 8).unwrap();
        renderer.render_frame().unwrap();
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].label, "mirror");
        assert_eq!(report.passes[0].outcome, PassOutcome::StaleTarget);
        assert_eq!(render_lines(&journal).len(), 1);

        // Auto-désactivée : la frame suivante la voit Disabled, sans
        // nouveau warn — puis update_pass la rebranche sur le handle frais.
        renderer.render_frame().unwrap();
        assert_eq!(
            renderer.frame_report().passes[0].outcome,
            PassOutcome::Disabled
        );
        renderer
            .update_pass(
                mirror,
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(fresh))
                    .with_order(-1),
            )
            .unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(
            renderer.frame_report().passes[0].outcome,
            PassOutcome::Executed
        );
        assert!(
            render_lines(&journal)
                .last()
                .is_some_and(|line| !line.contains("pass="))
        );
    }

    #[test]
    fn an_undeclared_feedback_draw_is_dropped() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let color = renderer.render_target_color(target).unwrap();
        let looping = renderer
            .create_material(
                &MaterialDescriptor::new("looping", MaterialModel::Unlit).with_texture(color),
            )
            .unwrap();
        let sane = plain_material(&mut renderer, "sane");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let textured_mesh = renderer.create_textured_mesh("quad", &screen_quad).unwrap();

        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh: textured_mesh,
                    material: looping,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh,
                    material: sane,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.render_frame().unwrap();
        // Le draw qui échantillonne la destination est écarté, l'autre passe.
        let mirror_line = render_lines(&journal)
            .into_iter()
            .find(|line| line.contains("pass=mirror"))
            .unwrap();
        assert_eq!(mirror_line.matches("m=[").count(), 1);
        assert_eq!(renderer.frame_report().passes[0].draws, 1);
    }

    #[test]
    fn an_empty_plan_still_flushes_the_retirement() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .set_pass_enabled(renderer.main_pass(), false)
            .unwrap();
        let material = plain_material(&mut renderer, "p");
        renderer.destroy_material(material).unwrap();
        assert_eq!(renderer.resource_stats().retired, 1);

        let outcome = renderer.render_frame().unwrap();
        assert_eq!(outcome, FrameOutcome::Rendered);
        assert_eq!(renderer.resource_stats().retired, 0);
        assert!(render_lines(&journal).is_empty());
        assert_eq!(
            renderer.frame_report().passes[0].outcome,
            PassOutcome::Disabled
        );
    }

    #[test]
    fn target_passes_survive_a_skipped_surface() {
        let (mut renderer, journal) =
            mock_renderer_with(FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
        let target = small_target(&mut renderer, "viewport");
        renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        let outcome = renderer.render_frame().unwrap();
        assert_eq!(outcome, FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
        // Le mock journalise tout le plan ; la vérité du rapport vient de
        // l'inférence du renderer : cible exécutée, surface sautée.
        assert!(!render_lines(&journal).is_empty());
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].outcome, PassOutcome::Executed);
        assert_eq!(report.passes[1].outcome, PassOutcome::SurfaceSkipped);
    }

    #[test]
    fn the_report_covers_the_orchestrated_frame_only() {
        let (mut renderer, _journal) = mock_renderer();
        assert!(renderer.frame_report().passes.is_empty());

        let target = small_target(&mut renderer, "viewport");
        renderer.render_frame().unwrap();
        assert_eq!(renderer.frame_report().passes.len(), 1);
        assert_eq!(renderer.frame_report().passes[0].label, "chaos.main");

        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap();
        assert_eq!(renderer.frame_report().passes.len(), 1);
    }

    #[test]
    fn render_to_target_is_the_offscreen_pass() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains(" dest=target0 pass=chaos.offscreen"));
    }

    #[test]
    fn the_declared_mirror_flow_is_the_checkpoint() {
        let (mut renderer, journal) = mock_renderer();

        // La cible, UN material de scène partagé par les deux passes (la
        // permutation offscreen se résout seule), le material qui
        // échantillonne sa couleur — le flux réel de la démo.
        let target = small_target(&mut renderer, "viewport");
        let scene_material = plain_material(&mut renderer, "scene");
        let color = renderer.render_target_color(target).unwrap();
        let screen_material = renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
            )
            .unwrap();
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let screen_mesh = renderer
            .create_textured_mesh("screen", &screen_quad)
            .unwrap();

        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_load(PassLoad::Clear(Color::rgb(0.1, 0.0, 0.2)))
                    .with_order(-10),
            )
            .unwrap();

        for _ in 0..2 {
            renderer.clear_draws();
            renderer
                .queue_draw_to(
                    mirror,
                    DrawCommand {
                        mesh,
                        material: scene_material,
                        transform: Transform::IDENTITY,
                    },
                )
                .unwrap();
            renderer.queue_draw(DrawCommand {
                mesh,
                material: scene_material,
                transform: Transform::IDENTITY,
            });
            renderer.queue_draw(DrawCommand {
                mesh: screen_mesh,
                material: screen_material,
                transform: Transform::IDENTITY,
            });
            renderer.render_frame().unwrap();
        }

        // Deux frames, le même ordre stable : la passe miroir (cible,
        // clear violet, le MÊME material que la scène) puis la
        // principale (scène + écran) — et le rapport les nomme.
        let lines = render_lines(&journal);
        assert_eq!(lines.len(), 4);
        for frame in lines.chunks(2) {
            assert!(frame[0].starts_with("render r=0.1 g=0 b=0.2 a=1"));
            assert!(frame[0].contains(" dest=target0 pass=mirror"));
            assert!(!frame[1].contains(" pass="));
            assert_eq!(frame[1].matches("m=[").count(), 2);
        }
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].label, "mirror");
        assert_eq!(report.passes[0].draws, 1);
        assert_eq!(report.passes[0].outcome, PassOutcome::Executed);
        assert_eq!(report.passes[1].label, "chaos.main");
        assert_eq!(report.passes[1].draws, 2);
        assert_eq!(report.passes[1].outcome, PassOutcome::Executed);

        // Les configurations incohérentes restent des erreurs explicites.
        assert!(renderer.add_pass(&surface_pass("mirror")).is_err());
        assert!(
            renderer
                .add_pass(
                    &RenderPassDescriptor::new("loop", RenderDestination::Target(target))
                        .with_reads(&[target]),
                )
                .is_err()
        );
    }

    #[test]
    fn same_model_and_state_share_one_pipeline() {
        let (mut renderer, journal) = mock_renderer();
        plain_material(&mut renderer, "a");
        plain_material(&mut renderer, "b");
        assert_eq!(create_pipeline_lines(&journal).len(), 1);
        assert!(
            create_pipeline_lines(&journal)[0]
                .starts_with("create_pipeline chaos.material.vertex_color ")
        );
    }

    #[test]
    fn each_state_is_its_own_permutation() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_material(&MaterialDescriptor::new(
                "culled",
                MaterialModel::VertexColor,
            ))
            .unwrap();
        renderer
            .create_material(
                &MaterialDescriptor::new("flat", MaterialModel::VertexColor).double_sided(),
            )
            .unwrap();
        renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let lines = create_pipeline_lines(&journal);
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("chaos.material.vertex_color.double_sided"));
        assert!(lines[2].contains("chaos.material.vertex_color.transparent"));
        assert!(lines[2].ends_with(" blend=alpha"));
    }

    #[test]
    fn one_material_serves_surface_and_target_passes() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let material = plain_material(&mut renderer, "scene");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        let command = DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        };
        renderer.queue_draw_to(mirror, command).unwrap();
        renderer.queue_draw(command);
        renderer.render_frame().unwrap();

        // L'eager surface (pipeline 0) + la permutation cible résolue au
        // rendu (pipeline 1, target=…) — UN material, deux passes, zéro
        // duplication déclarée par le consommateur.
        let pipelines = create_pipeline_lines(&journal);
        assert_eq!(pipelines.len(), 2);
        assert!(pipelines[1].contains("target=Rgba8UnormSrgb"));
        let lines = render_lines(&journal);
        assert!(lines[0].contains("dest=target0"));
        assert!(lines[0].contains("draws=[(1,"));
        assert!(lines[1].contains("draws=[(0,"));
    }

    #[test]
    fn a_custom_model_resolves_or_fails_at_creation() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_material(&MaterialDescriptor::new(
                "toon",
                MaterialModel::Custom {
                    shader: ShaderRef::Inline(ShaderSource::Wgsl(String::from("custom-code"))),
                    vertex_layout: ColorVertex::layout(),
                    material_inputs: false,
                },
            ))
            .unwrap();
        assert!(
            create_pipeline_lines(&journal)[0]
                .starts_with("create_pipeline chaos.material.custom.inline code=custom-code")
        );

        let missing = renderer
            .create_material(&MaterialDescriptor::new(
                "broken",
                MaterialModel::Custom {
                    shader: ShaderRef::from("game.missing"),
                    vertex_layout: ColorVertex::layout(),
                    material_inputs: false,
                },
            ))
            .unwrap_err();
        assert!(missing.to_string().contains("not found in the library"));
    }

    #[test]
    fn a_mismatched_vertex_layout_drops_the_draw() {
        let (mut renderer, journal) = mock_renderer();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        let material = textured_material(&mut renderer, "m", texture, sampler);
        let wrong_mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let right_mesh = renderer
            .create_textured_mesh("quad", &quad_geometry)
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: wrong_mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh: right_mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        // Le mesh ColorVertex est écarté (le modèle Unlit attend
        // TexturedVertex), le mesh assorti passe.
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
        assert_eq!(renderer.frame_report().passes[0].draws, 1);
    }

    #[test]
    fn inputs_on_an_inputless_model_are_refused() {
        let (mut renderer, _journal) = mock_renderer();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        let with_texture = renderer
            .create_material(
                &MaterialDescriptor::new("t", MaterialModel::VertexColor).with_texture(texture),
            )
            .unwrap_err();
        assert!(with_texture.to_string().contains("no material inputs"));
        let with_sampler = renderer
            .create_material(
                &MaterialDescriptor::new("s", MaterialModel::VertexColor).with_sampler(sampler),
            )
            .unwrap_err();
        assert!(with_sampler.to_string().contains("no material inputs"));
        let with_color = renderer
            .create_material(
                &MaterialDescriptor::new("c", MaterialModel::VertexColor)
                    .with_base_color(Color::rgb(1.0, 0.0, 0.0)),
            )
            .unwrap_err();
        assert!(with_color.to_string().contains("base_color"));

        let plain = plain_material(&mut renderer, "p");
        let set_color = renderer
            .set_material_color(plain, Color::rgb(1.0, 0.0, 0.0))
            .unwrap_err();
        assert!(set_color.to_string().contains("no material inputs"));
        let set_texture = renderer
            .set_material_texture(plain, Some(texture))
            .unwrap_err();
        assert!(set_texture.to_string().contains("no material inputs"));
    }

    #[test]
    fn set_material_color_writes_in_place() {
        let (mut renderer, journal) = mock_renderer();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        let material = textured_material(&mut renderer, "m", texture, sampler);
        let bindings_before = journal
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .count();
        let pipelines_before = create_pipeline_lines(&journal).len();

        renderer
            .set_material_color(material, Color::rgb(0.9, 0.1, 0.2))
            .unwrap();

        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "update_material_binding index=0 color=(0.9, 0.1, 0.2, 1)"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("create_material_binding"))
                .count(),
            bindings_before
        );
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        let info = renderer.material_info(material).unwrap();
        assert_eq!(info.base_color, Color::rgb(0.9, 0.1, 0.2));

        renderer.destroy_material(material).unwrap();
        let stale = renderer
            .set_material_color(material, Color::WHITE)
            .unwrap_err();
        assert!(stale.to_string().contains("stale"));
    }

    #[test]
    fn set_material_texture_swaps_transactionally() {
        let (mut renderer, journal) = mock_renderer();
        let first = small_texture(&mut renderer, "first");
        let second = small_texture(&mut renderer, "second");
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        let material = textured_material(&mut renderer, "m", first, sampler);

        renderer
            .set_material_texture(material, Some(second))
            .unwrap();

        // L'ancienne texture est rendue (destructible), la nouvelle est
        // partagée (refusée), le handle du material SURVIT.
        renderer.destroy_texture(first).unwrap();
        let still_used = renderer.destroy_texture(second).unwrap_err();
        assert!(still_used.to_string().contains("1 material(s)"));
        assert_eq!(renderer.material_info(material).unwrap().texture, second);

        // L'ancien binding part en retraite, vidée au point sûr.
        renderer.render_frame().unwrap();
        assert!(
            journal
                .entries()
                .contains(&String::from("destroy_material_binding index=0"))
        );

        // La même texture est un no-op : aucun nouveau binding.
        let bindings_before = journal
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .count();
        renderer
            .set_material_texture(material, Some(second))
            .unwrap();
        assert_eq!(
            journal
                .entries()
                .iter()
                .filter(|entry| entry.starts_with("create_material_binding"))
                .count(),
            bindings_before
        );

        // Un cubemap reste refusé au même titre qu'à la création.
        let cube = renderer
            .create_texture(&TextureDescriptor::cube(
                "env",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 24],
            ))
            .unwrap();
        let refused = renderer
            .set_material_texture(material, Some(cube))
            .unwrap_err();
        assert!(refused.to_string().contains("cubemap"));
    }

    #[test]
    fn opaque_draws_come_before_transparent_ones() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let solid = plain_material(&mut renderer, "solid");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        // Le transparent est SOUMIS d'abord — le plan le rend en dernier.
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        });
        renderer.queue_draw(DrawCommand {
            mesh,
            material: solid,
            transform: Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        let opaque_at = line.find("m=[2, 0, 0]").unwrap();
        let transparent_at = line.find("m=[1, 0, 0]").unwrap();
        assert!(opaque_at < transparent_at);

        // La même partition s'applique au rendu immédiat vers une cible
        // — avec la caméra large : la vignette cull avec SA vue.
        let target = small_target(&mut renderer, "viewport");
        renderer
            .render_to_target(
                target,
                Color::BLACK,
                Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)),
                &[
                    DrawCommand {
                        mesh,
                        material: glass,
                        transform: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
                    },
                    DrawCommand {
                        mesh,
                        material: solid,
                        transform: Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
                    },
                ],
            )
            .unwrap();
        let offscreen = render_lines(&journal).pop().unwrap();
        let opaque_at = offscreen.find("m=[2, 0, 0]").unwrap();
        let transparent_at = offscreen.find("m=[1, 0, 0]").unwrap();
        assert!(opaque_at < transparent_at);
    }

    #[test]
    fn material_info_is_the_full_inspection() {
        let (mut renderer, _journal) = mock_renderer();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("inspected", MaterialModel::Unlit)
                    .double_sided()
                    .with_opacity(MaterialOpacity::Transparent)
                    .with_base_color(Color::rgb(0.5, 0.25, 1.0))
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let info = renderer.material_info(material).unwrap();
        assert_eq!(info.label, "inspected");
        assert_eq!(info.model, MaterialModel::Unlit);
        assert_eq!(info.base_color, Color::rgb(0.5, 0.25, 1.0));
        assert_eq!(info.texture, texture);
        assert_eq!(info.sampler, sampler);
        assert!(info.double_sided);
        assert_eq!(info.opacity, MaterialOpacity::Transparent);

        renderer.destroy_material(material).unwrap();
        assert!(
            renderer
                .material_info(material)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
    }

    #[test]
    fn stats_count_lazy_permutations_and_bindings() {
        let (mut renderer, _journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let material = plain_material(&mut renderer, "scene");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let stats = renderer.resource_stats();
        assert_eq!(stats.pipelines, 1);
        assert_eq!(stats.materials, 1);
        assert_eq!(stats.bindings, 1);

        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.render_frame().unwrap();
        // La permutation cible résolue au rendu est comptée aux stats.
        assert_eq!(renderer.resource_stats().pipelines, 2);
    }

    #[test]
    fn a_feedback_introduced_by_update_is_caught() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let texture = small_texture(&mut renderer, "innocent");
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(texture),
            )
            .unwrap();
        let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let mesh = renderer
            .create_textured_mesh("quad", &quad_geometry)
            .unwrap();
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(renderer.frame_report().passes[0].draws, 1);

        // Le material se met à échantillonner la cible de SA passe : le
        // resolve suivant l'écarte.
        let color = renderer.render_target_color(target).unwrap();
        renderer
            .set_material_texture(material, Some(color))
            .unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(renderer.frame_report().passes[0].draws, 0);
        let _ = journal;
    }

    #[test]
    fn the_material_system_checkpoint() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let checker = small_texture(&mut renderer, "checker");
        // Deux meshes partagent le material A ; un troisième porte le
        // material B ; un quatrième au MAUVAIS layout est écarté.
        let shared = renderer
            .create_material(
                &MaterialDescriptor::new("shared", MaterialModel::Unlit).with_texture(checker),
            )
            .unwrap();
        let tinted = renderer
            .create_material(
                &MaterialDescriptor::new("tinted", MaterialModel::Unlit)
                    .with_base_color(Color::rgb(0.2, 0.4, 0.8)),
            )
            .unwrap();
        let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let mesh_a = renderer.create_textured_mesh("a", &quad_geometry).unwrap();
        let mesh_b = renderer.create_textured_mesh("b", &quad_geometry).unwrap();
        let mesh_c = renderer.create_textured_mesh("c", &quad_geometry).unwrap();
        let wrong = renderer.create_mesh("wrong", &triangle()).unwrap();
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-10),
            )
            .unwrap();

        for _ in 0..2 {
            renderer.clear_draws();
            for mesh in [mesh_a, mesh_b] {
                renderer.queue_draw(DrawCommand {
                    mesh,
                    material: shared,
                    transform: Transform::IDENTITY,
                });
            }
            renderer.queue_draw(DrawCommand {
                mesh: mesh_c,
                material: tinted,
                transform: Transform::IDENTITY,
            });
            renderer.queue_draw(DrawCommand {
                mesh: wrong,
                material: tinted,
                transform: Transform::IDENTITY,
            });
            renderer
                .queue_draw_to(
                    mirror,
                    DrawCommand {
                        mesh: mesh_a,
                        material: shared,
                        transform: Transform::IDENTITY,
                    },
                )
                .unwrap();
            renderer.render_frame().unwrap();
            // Entre les deux frames : la couleur de B change SANS
            // recréation (le chemin contrôlé).
            renderer
                .set_material_color(tinted, Color::rgb(0.9, 0.9, 0.1))
                .unwrap();
        }

        let report = renderer.frame_report();
        assert_eq!(report.passes[0].label, "mirror");
        assert_eq!(report.passes[0].draws, 1);
        assert_eq!(report.passes[1].label, "chaos.main");
        assert_eq!(report.passes[1].draws, 3);

        let entries = journal.entries();
        // Deux materials Unlit au même état = UNE permutation surface +
        // UNE permutation cible ; deux bindings ; deux updates in-place ;
        // aucun binding recréé.
        assert_eq!(create_pipeline_lines(&journal).len(), 2);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("create_material_binding"))
                .count(),
            2
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("update_material_binding"))
                .count(),
            2
        );
    }

    fn lights_lines(journal: &Journal) -> Vec<String> {
        journal
            .entries()
            .into_iter()
            .filter(|entry| entry.starts_with("lights "))
            .collect()
    }

    fn lit_quad_mesh(renderer: &mut Renderer, label: &str) -> MeshHandle {
        let quad = LitGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        renderer.create_lit_mesh(label, &quad).unwrap()
    }

    #[test]
    fn the_lit_vertex_declares_its_layout() {
        let layout = LitVertex::layout();
        assert_eq!(layout.stride, 32);
        assert_eq!(layout.attributes.len(), 3);
        assert_eq!(layout.attributes[1].offset, 12);
        assert_eq!(layout.attributes[2].offset, 24);
        let vertex = LitVertex {
            position: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5, 0.25],
        };
        let bytes = LitVertex::bytes_of(&[vertex]);
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[12..16], 0.0f32.to_ne_bytes());
        assert_eq!(bytes[16..20], 1.0f32.to_ne_bytes());
    }

    #[test]
    fn lit_geometry_keeps_its_face_normals() {
        let cube = LitGeometry::cube([0.0, 0.0, 0.0], 2.0);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
        for vertex in &cube.vertices {
            let length: f32 = vertex
                .normal
                .iter()
                .map(|component| component * component)
                .sum();
            assert!((length - 1.0).abs() < 1e-6);
        }
        // La face +Y (sommets 8..12) porte la normale +Y exacte.
        assert_eq!(cube.vertices[8].normal, [0.0, 1.0, 0.0]);
        let quad = LitGeometry::quad([0.0, 0.0, 0.0], 2.0, 2.0, 1.0);
        for vertex in &quad.vertices {
            assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn a_lit_material_resolves_the_lit_permutation() {
        let (mut renderer, journal) = mock_renderer();
        let material = renderer
            .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Lit))
            .unwrap();
        assert!(
            create_pipeline_lines(&journal)[0].starts_with("create_pipeline chaos.material.lit ")
        );
        let mesh = lit_quad_mesh(&mut renderer, "quad");
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
    }

    #[test]
    fn a_textured_mesh_under_a_lit_material_is_dropped() {
        let (mut renderer, journal) = mock_renderer();
        let material = renderer
            .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Lit))
            .unwrap();
        let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
        let wrong = renderer.create_textured_mesh("quad", &quad).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: wrong,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.ends_with("draws=[]"));
    }

    #[test]
    fn submitted_lights_reach_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_ambient_light(Color::rgb(1.0, 1.0, 1.0), 0.1);
        renderer.submit_light(Light::directional(
            Vec3::new(0.0, -2.0, 0.0),
            Color::rgb(1.0, 0.9, 0.8),
            0.9,
        ));
        renderer.submit_light(Light::point(
            Vec3::new(1.0, 2.0, 3.0),
            Color::rgb(1.0, 0.0, 0.0),
            2.5,
            5.0,
        ));
        renderer.render_frame().unwrap();
        let lines = lights_lines(&journal);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("lights ambient=(1, 1, 1, 0.1) count=2"));
        // La direction soumise (0, -2, 0) arrive NORMALISÉE au backend.
        assert!(lines[0].contains("[directional d=(0, -1, 0) c=(1, 0.9, 0.8) i=0.9]"));
        assert!(lines[0].contains("[point p=(1, 2, 3) r=5 c=(1, 0, 0) i=2.5]"));
    }

    #[test]
    fn an_unlit_frame_emits_no_lights_line() {
        let (mut renderer, journal) = mock_renderer();
        renderer.render_frame().unwrap();
        assert!(lights_lines(&journal).is_empty());
    }

    #[test]
    fn lights_overflow_is_truncated_predictably() {
        let (mut renderer, journal) = mock_renderer();
        for index in 0..20 {
            renderer.submit_light(Light::point(
                Vec3::new(f32::from(u8::try_from(index).unwrap_or(0)), 0.0, 0.0),
                Color::WHITE,
                1.0,
                5.0,
            ));
        }
        renderer.render_frame().unwrap();
        let line = lights_lines(&journal).pop().unwrap();
        assert!(line.contains("count=16"));
        // L'ordre de soumission est préservé : la première gagne, la
        // dix-septième (x=16) n'entre pas.
        assert!(line.contains("[point p=(0, 0, 0)"));
        assert!(line.contains("p=(15, 0, 0)"));
        assert!(!line.contains("p=(16, 0, 0)"));
        // Sous la limite à la frame suivante : l'épisode se réarme.
        renderer.clear_draws();
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        assert!(lights_lines(&journal).pop().unwrap().contains("count=1"));
        assert!(!renderer.lights_truncation_warned);
    }

    #[test]
    fn disabled_and_invalid_lights_are_excluded() {
        let (mut renderer, journal) = mock_renderer();
        let mut off = Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0);
        off.set_enabled(false);
        renderer.submit_light(off);
        // Invalides : écartées AU SUBMIT, jamais stockées.
        renderer.submit_light(Light::directional(Vec3::ZERO, Color::WHITE, 1.0));
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, -1.0, 5.0));
        renderer.submit_light(Light::spot(
            Vec3::ZERO,
            Vec3::NEG_Y,
            Color::WHITE,
            1.0,
            5.0,
            0.4,
            0.4,
        ));
        renderer.submit_light(Light::point(Vec3::X, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        let line = lights_lines(&journal).pop().unwrap();
        assert!(line.contains("count=1"));
        assert!(line.contains("p=(1, 0, 0)"));
    }

    #[test]
    fn clear_draws_clears_lights_but_keeps_the_ambient() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_ambient_light(Color::WHITE, 0.2);
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        assert!(lights_lines(&journal).pop().unwrap().contains("count=1"));

        renderer.clear_draws();
        renderer.render_frame().unwrap();
        // Les lumières sont re-soumises chaque frame ; l'ambiante est un
        // réglage persistant.
        let line = lights_lines(&journal).pop().unwrap();
        assert!(line.contains("count=0"));
        assert!(line.contains("ambient=(1, 1, 1, 0.2)"));
        assert_eq!(renderer.ambient_light(), (Color::WHITE, 0.2));
    }

    #[test]
    fn the_frame_lights_serve_every_pass() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        // UNE ligne lights pour le plan entier : les deux passes
        // partagent le même éclairage.
        assert_eq!(lights_lines(&journal).len(), 1);
        assert_eq!(render_lines(&journal).len(), 2);
    }

    #[test]
    fn render_to_target_carries_the_collected_lights() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let mut off = Light::point(Vec3::X, Color::WHITE, 1.0, 5.0);
        off.set_enabled(false);
        renderer.submit_light(off);
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap();
        // Le chemin immédiat passe par la MÊME collection : la
        // désactivée est filtrée là aussi.
        let line = lights_lines(&journal).pop().unwrap();
        assert!(line.contains("count=1"));
        assert!(line.contains("p=(0, 0, 0)"));
    }

    #[test]
    fn an_empty_plan_sends_no_lights() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .set_pass_enabled(renderer.main_pass(), false)
            .unwrap();
        renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        assert!(lights_lines(&journal).is_empty());
        assert!(render_lines(&journal).is_empty());
    }

    #[test]
    fn the_lighting_checkpoint() {
        let (mut renderer, journal) = mock_renderer();
        // La scène de validation : un material Lit sur mesh éclairable,
        // une directionnelle + deux ponctuelles + un spot.
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("shaded", MaterialModel::Lit)
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "quad");
        renderer.set_ambient_light(Color::WHITE, 0.05);

        for frame in 0..2 {
            renderer.clear_draws();
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
            let mut sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0);
            // La deuxième frame éteint le soleil : le toggle observable.
            sun.set_enabled(frame == 0);
            renderer.submit_light(sun);
            renderer.submit_light(Light::point(Vec3::X, Color::rgb(1.0, 0.0, 0.0), 2.0, 4.0));
            renderer.submit_light(Light::point(Vec3::Z, Color::rgb(0.0, 0.0, 1.0), 2.0, 4.0));
            renderer.submit_light(Light::spot(
                Vec3::Y,
                Vec3::NEG_Y,
                Color::WHITE,
                3.0,
                8.0,
                0.2,
                0.4,
            ));
            renderer.render_frame().unwrap();
        }

        let lines = lights_lines(&journal);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("count=4"));
        assert!(lines[0].contains("[directional"));
        assert!(lines[0].contains("[spot p=(0, 1, 0) d=(0, -1, 0) r=8]"));
        assert!(lines[1].contains("count=3"));
        assert!(!lines[1].contains("[directional"));
        // Le draw éclairé est résolu dans les deux frames.
        for line in render_lines(&journal) {
            assert_eq!(line.matches("m=[").count(), 1);
        }
    }

    fn binding_lines(journal: &Journal) -> Vec<String> {
        journal
            .entries()
            .into_iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .collect()
    }

    #[test]
    fn pbr_properties_are_refused_on_non_pbr_models() {
        let (mut renderer, _journal) = mock_renderer();
        let refused = renderer
            .create_material(&MaterialDescriptor::new("m", MaterialModel::Lit).with_metallic(0.5))
            .unwrap_err();
        assert!(refused.to_string().contains("does not consume PBR"));
        assert!(refused.to_string().contains("'metallic'"));
        let (texture, _sampler) = texture_and_sampler(&mut renderer);
        let mapped = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Unlit).with_normal_map(texture),
            )
            .unwrap_err();
        assert!(mapped.to_string().contains("'normal_map'"));

        // Acceptées sur Pbr et sur Custom-avec-inputs (le shader custom
        // voit tout le groupe 2 — délégation documentée).
        renderer
            .create_material(
                &MaterialDescriptor::new("p", MaterialModel::Pbr)
                    .with_metallic(0.5)
                    .with_roughness(0.3),
            )
            .unwrap();
        renderer
            .create_material(
                &MaterialDescriptor::new(
                    "c",
                    MaterialModel::Custom {
                        shader: ShaderRef::Inline(ShaderSource::Wgsl(String::from("custom-code"))),
                        vertex_layout: LitVertex::layout(),
                        material_inputs: true,
                    },
                )
                .with_emissive(Color::rgb(1.0, 0.0, 0.0)),
            )
            .unwrap();
    }

    #[test]
    fn receive_shadows_off_is_refused_on_unlit_models() {
        let (mut renderer, journal) = mock_renderer();
        let vertex_color = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::VertexColor).without_shadow_receive(),
            )
            .unwrap_err();
        assert!(
            vertex_color
                .to_string()
                .contains("does not react to lighting")
        );
        assert!(vertex_color.to_string().contains("'receive_shadows'"));
        let unlit = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Unlit).without_shadow_receive(),
            )
            .unwrap_err();
        assert!(unlit.to_string().contains("does not react to lighting"));

        // Accepté sur les modèles éclairés — et le binding le journalise
        // hors défaut. cast_shadows n'a pas de contrainte de modèle (une
        // géométrie projette quel que soit son shader).
        renderer
            .create_material(
                &MaterialDescriptor::new("l", MaterialModel::Lit).without_shadow_receive(),
            )
            .unwrap();
        assert!(binding_lines(&journal).pop().unwrap().contains("recv=off"));
        renderer
            .create_material(
                &MaterialDescriptor::new("v", MaterialModel::VertexColor).without_shadow_cast(),
            )
            .unwrap();
    }

    #[test]
    fn masked_contracts_are_validated_at_creation() {
        let (mut renderer, _journal) = mock_renderer();
        // Masked sans entrées material : aucun alpha à tester — refusé
        // en nommant la règle.
        let blind = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::VertexColor)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap_err();
        assert!(blind.to_string().contains("no alpha to test"));
        // Le cutoff hors défaut est réservé à Masked.
        let inert = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Lit).with_alpha_cutoff(0.3),
            )
            .unwrap_err();
        assert!(inert.to_string().contains("'alpha_cutoff'"));
        // Les bornes du cutoff sont nommées.
        let wild = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked)
                    .with_alpha_cutoff(1.5),
            )
            .unwrap_err();
        assert!(wild.to_string().contains("0..=1"));
        // Accepté sur les modèles à entrées — Unlit, Lit, Pbr et le
        // Custom délégué (son shader doit exposer fs_masked).
        renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked)
                    .with_alpha_cutoff(0.35),
            )
            .unwrap();
        let probe = renderer
            .create_material(
                &MaterialDescriptor::new("probe", MaterialModel::Unlit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let info = renderer.material_info(probe).unwrap();
        assert_eq!(info.opacity, MaterialOpacity::Masked);
        assert_eq!(info.alpha_cutoff, 0.5);
    }

    #[test]
    fn the_masked_permutation_has_its_own_pipeline_and_entry() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        // Même modèle, opacités différentes : DEUX permutations — la
        // masked porte son suffixe de label et son entrée fs_masked.
        let lines = create_pipeline_lines(&journal);
        assert!(lines[0].starts_with("create_pipeline chaos.material.lit "));
        assert!(!lines[0].contains(" entry="));
        assert!(lines[1].starts_with("create_pipeline chaos.material.lit.masked "));
        assert!(lines[1].contains(" entry=fs_masked"));
        assert!(!lines[1].contains(" blend=alpha"));
        // Un second material masked du même modèle réutilise la
        // permutation (le cache déduplique).
        renderer
            .create_material(
                &MaterialDescriptor::new("fence", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        assert_eq!(create_pipeline_lines(&journal).len(), 2);
    }

    #[test]
    fn the_alpha_cutoff_updates_in_place() {
        let (mut renderer, journal) = mock_renderer();
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked)
                    .with_alpha_cutoff(0.35),
            )
            .unwrap();
        assert!(
            binding_lines(&journal)
                .pop()
                .unwrap()
                .contains(" cutoff=0.35")
        );
        let bindings_before = binding_lines(&journal).len();
        let pipelines_before = create_pipeline_lines(&journal).len();
        renderer.set_material_alpha_cutoff(grid, 0.7).unwrap();
        // L'écriture est IN-PLACE : un update au journal, aucun binding
        // ni pipeline créé, l'inspection reflète la valeur.
        assert!(
            journal
                .entries()
                .iter()
                .rfind(|entry| entry.starts_with("update_material_binding"))
                .unwrap()
                .contains(" cutoff=0.7")
        );
        assert_eq!(binding_lines(&journal).len(), bindings_before);
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        assert_eq!(renderer.material_info(grid).unwrap().alpha_cutoff, 0.7);
        // Refus nommés : hors Masked, hors bornes.
        let solid = renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        let refused = renderer.set_material_alpha_cutoff(solid, 0.3).unwrap_err();
        assert!(refused.to_string().contains("is not Masked"));
        let wild = renderer.set_material_alpha_cutoff(grid, 2.0).unwrap_err();
        assert!(wild.to_string().contains("0..=1"));
    }

    #[test]
    fn transparent_draws_are_sorted_back_to_front() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for z in [-1.0, -5.0, -3.0] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        // Caméra à l'origine (défaut) : le plus LOINTAIN d'abord.
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        let far = line.find("m=[0, 0, -5]").unwrap();
        let mid = line.find("m=[0, 0, -3]").unwrap();
        let near = line.find("m=[0, 0, -1]").unwrap();
        assert!(far < mid && mid < near);
    }

    #[test]
    fn the_transparent_sort_follows_the_camera() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for z in [-1.0, -5.0] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        renderer.set_camera_position(Vec3::new(0.0, 0.0, 2.0));
        renderer.render_frame().unwrap();
        let first = render_lines(&journal).pop().unwrap();
        assert!(first.find("m=[0, 0, -5]").unwrap() < first.find("m=[0, 0, -1]").unwrap());
        // La caméra passe DERRIÈRE les panneaux : l'ordre s'inverse à la
        // frame suivante — le tri suit la caméra de la passe.
        renderer.set_camera_position(Vec3::new(0.0, 0.0, -8.0));
        renderer.render_frame().unwrap();
        let second = render_lines(&journal).pop().unwrap();
        assert!(second.find("m=[0, 0, -1]").unwrap() < second.find("m=[0, 0, -5]").unwrap());
    }

    #[test]
    fn equal_depths_keep_the_submission_order() {
        let (mut renderer, journal) = mock_renderer();
        let veil = renderer
            .create_material(
                &MaterialDescriptor::new("veil", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let tint = renderer
            .create_material(
                &MaterialDescriptor::new("tint", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        // Deux transparents à la MÊME distance : le tri stable conserve
        // l'ordre d'arrivée (regroupé par material par la file).
        for material in [tint, veil] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, -2.0)),
            });
        }
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.find("b=Some(0)").unwrap() < line.find("b=Some(1)").unwrap());
    }

    #[test]
    fn the_pass_order_is_opaque_masked_sky_transparent() {
        let (mut renderer, journal) = mock_renderer();
        let cubemap = renderer
            .create_texture(&TextureDescriptor::cube(
                "sky.cube",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 4 * 6],
            ))
            .unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        let solid = renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        // Soumis à l'ENVERS de l'ordre de rendu : la partition remet
        // opaque → masked → ciel → transparent.
        for material in [glass, grid, solid] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();
        // Bindings dans l'ordre de création : solid=0, grid=1, glass=2 ;
        // le ciel est le tuple sans buffers (pipeline lazy, 3 sommets).
        let line = render_lines(&journal).pop().unwrap();
        let solid_at = line.find("b=Some(0)").unwrap();
        let grid_at = line.find("b=Some(1)").unwrap();
        let sky_at = line.find(", None, None, 3, b=None").unwrap();
        let glass_at = line.find("b=Some(2)").unwrap();
        assert!(solid_at < grid_at);
        assert!(grid_at < sky_at);
        assert!(sky_at < glass_at);
    }

    #[test]
    fn the_breakdown_reports_the_categories() {
        let (mut renderer, _journal) = mock_renderer();
        let cubemap = renderer
            .create_texture(&TextureDescriptor::cube(
                "sky.cube",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 4 * 6],
            ))
            .unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        let solid = renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for material in [solid, solid, grid, glass] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();
        let report = renderer.frame_report();
        assert_eq!(
            report.passes[0].breakdown,
            DrawBreakdown {
                opaque: 2,
                masked: 1,
                transparent: 1,
                injected: 1,
            }
        );
        assert_eq!(report.passes[0].draws, 5);
        // Une passe désactivée rapporte une ventilation vide.
        let main = renderer.main_pass();
        renderer.set_pass_enabled(main, false).unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(
            renderer.frame_report().passes[0].breakdown,
            DrawBreakdown::default()
        );
    }

    #[test]
    fn masked_materials_cast_shadow_silhouettes() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for material in [grid, glass] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();
        // Le masked projette (sa silhouette pleine V1), le transparent
        // jamais — le contrat de la catégorie.
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );
    }

    #[test]
    fn material_info_reflects_the_shadow_flags() {
        let (mut renderer, _journal) = mock_renderer();
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Pbr)
                    .without_shadow_cast()
                    .without_shadow_receive(),
            )
            .unwrap();
        let info = renderer.material_info(material).unwrap();
        assert!(!info.cast_shadows);
        assert!(!info.receive_shadows);
        let lit = renderer
            .create_material(&MaterialDescriptor::new("d", MaterialModel::Lit))
            .unwrap();
        let info = renderer.material_info(lit).unwrap();
        assert!(info.cast_shadows);
        assert!(info.receive_shadows);
    }

    #[test]
    fn a_pbr_material_resolves_its_permutation_and_draws() {
        let (mut renderer, journal) = mock_renderer();
        let material = renderer
            .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Pbr))
            .unwrap();
        assert!(
            create_pipeline_lines(&journal)[0].starts_with("create_pipeline chaos.material.pbr ")
        );
        let mesh = lit_quad_mesh(&mut renderer, "quad");
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
    }

    #[test]
    fn the_binding_carries_the_seven_slots() {
        let (mut renderer, journal) = mock_renderer();
        let mr = small_texture(&mut renderer, "mr");
        let normal = small_texture(&mut renderer, "bumps");
        renderer
            .create_material(
                &MaterialDescriptor::new("full", MaterialModel::Pbr)
                    .with_metallic(0.7)
                    .with_roughness(0.25)
                    .with_metallic_roughness_texture(mr)
                    .with_normal_map(normal)
                    .with_emissive(Color::rgb(2.0, 1.0, 0.5)),
            )
            .unwrap();
        let line = binding_lines(&journal).pop().unwrap();
        // mr/normal explicites (idx 0 et 1), ao/émissif en fallback
        // blanc (idx 2), base en fallback blanc aussi.
        assert!(line.contains("texture=2"));
        assert!(line.contains(" mr=0 "));
        assert!(line.contains(" normal=1 "));
        assert!(line.contains(" ao=2 "));
        assert!(line.contains(" em=2"));
        assert!(line.contains(" metallic=0.7"));
        assert!(line.contains(" roughness=0.25"));
        assert!(line.contains(" emissive=(2, 1, 0.5)"));
    }

    #[test]
    fn cubemaps_are_refused_on_every_pbr_slot() {
        let (mut renderer, _journal) = mock_renderer();
        let cube = renderer
            .create_texture(&TextureDescriptor::cube(
                "env",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 24],
            ))
            .unwrap();
        let slots: [fn(MaterialDescriptor, TextureHandle) -> MaterialDescriptor; 4] = [
            |d, t| d.with_metallic_roughness_texture(t),
            |d, t| d.with_normal_map(t),
            |d, t| d.with_occlusion_texture(t),
            |d, t| d.with_emissive_texture(t),
        ];
        for attach in slots {
            let descriptor = attach(MaterialDescriptor::new("m", MaterialModel::Pbr), cube);
            let refused = renderer.create_material(&descriptor).unwrap_err();
            assert!(refused.to_string().contains("cubemap"));
        }
    }

    #[test]
    fn every_pbr_slot_is_refcounted() {
        let (mut renderer, _journal) = mock_renderer();
        let mr = small_texture(&mut renderer, "mr");
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Pbr)
                    .with_metallic_roughness_texture(mr),
            )
            .unwrap();
        let refused = renderer.destroy_texture(mr).unwrap_err();
        assert!(refused.to_string().contains("still used by 1 material(s)"));
        renderer.destroy_material(material).unwrap();
        renderer.destroy_texture(mr).unwrap();
    }

    #[test]
    fn a_doubled_slot_takes_two_shares() {
        let (mut renderer, _journal) = mock_renderer();
        let shared = small_texture(&mut renderer, "both");
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("m", MaterialModel::Pbr)
                    .with_texture(shared)
                    .with_emissive_texture(shared),
            )
            .unwrap();
        // Deux slots = deux parts — la symétrie exacte du release.
        let refused = renderer.destroy_texture(shared).unwrap_err();
        assert!(refused.to_string().contains("still used by 2 material(s)"));
        renderer.destroy_material(material).unwrap();
        renderer.destroy_texture(shared).unwrap();
    }

    #[test]
    fn pbr_params_update_in_place() {
        let (mut renderer, journal) = mock_renderer();
        let material = renderer
            .create_material(&MaterialDescriptor::new("m", MaterialModel::Pbr))
            .unwrap();
        let bindings_before = binding_lines(&journal).len();
        let pipelines_before = create_pipeline_lines(&journal).len();

        renderer.set_material_metallic(material, 0.9).unwrap();
        renderer.set_material_roughness(material, 0.2).unwrap();
        renderer
            .set_material_emissive(material, Color::rgb(3.0, 1.5, 0.0))
            .unwrap();

        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "update_material_binding index=0 color=(1, 1, 1, 1) metallic=0.9 roughness=0.2 emissive=(3, 1.5, 0)"
        );
        assert_eq!(binding_lines(&journal).len(), bindings_before);
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);

        let info = renderer.material_info(material).unwrap();
        assert_eq!(info.metallic, 0.9);
        assert_eq!(info.roughness, 0.2);
        assert_eq!(info.emissive, Color::rgb(3.0, 1.5, 0.0));

        // Refusé sur un modèle qui ne consomme pas les propriétés PBR.
        let lit = renderer
            .create_material(&MaterialDescriptor::new("lit", MaterialModel::Lit))
            .unwrap();
        let refused = renderer.set_material_metallic(lit, 0.5).unwrap_err();
        assert!(refused.to_string().contains("does not consume PBR"));

        renderer.destroy_material(material).unwrap();
        let stale = renderer.set_material_roughness(material, 0.5).unwrap_err();
        assert!(stale.to_string().contains("stale"));
    }

    #[test]
    fn a_pbr_slot_feedback_is_dropped() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let color = renderer.render_target_color(target).unwrap();
        let material = renderer
            .create_material(
                &MaterialDescriptor::new("glow", MaterialModel::Pbr).with_emissive_texture(color),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "quad");
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.render_frame().unwrap();
        // Le slot émissif porte la couleur de la cible de SA passe : le
        // feedback est attrapé même hors du slot de base.
        assert_eq!(renderer.frame_report().passes[0].draws, 0);
        let _ = journal;
    }

    #[test]
    fn the_camera_position_travels_per_pass() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_camera_position(Vec3::new(0.0, 7.0, 0.0))
                    .with_order(-1),
            )
            .unwrap();
        renderer.set_camera_position(Vec3::new(1.0, 2.0, 3.0));
        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        assert!(lines[0].ends_with(" cam=(0, 7, 0)"));
        assert!(lines[1].ends_with(" cam=(1, 2, 3)"));

        // Le setter par passe écrase ; ZERO = pas de suffixe (le défaut).
        renderer
            .set_pass_camera_position(mirror, Vec3::ZERO)
            .unwrap();
        renderer.set_camera_position(Vec3::ZERO);
        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        assert!(!lines[2].contains("cam="));
        assert!(!lines[3].contains("cam="));

        // Le rendu immédiat n'a pas de caméra (documenté : ZERO).
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap();
        assert!(!render_lines(&journal).pop().unwrap().contains("cam="));
    }

    #[test]
    fn the_pbr_checkpoint() {
        let (mut renderer, journal) = mock_renderer();
        let normal_map = small_texture(&mut renderer, "bumps");
        let mesh = lit_quad_mesh(&mut renderer, "sphere");

        // La grille : des combinaisons metallic/roughness distinctes qui
        // partagent UNE permutation, plus une normal map et un émissif.
        let mut materials = Vec::new();
        for (index, (metallic, roughness)) in [(0.0, 0.1), (0.0, 1.0), (1.0, 0.1), (1.0, 1.0)]
            .iter()
            .enumerate()
        {
            materials.push(
                renderer
                    .create_material(
                        &MaterialDescriptor::new(format!("grid.{index}"), MaterialModel::Pbr)
                            .with_metallic(*metallic)
                            .with_roughness(*roughness),
                    )
                    .unwrap(),
            );
        }
        let bumpy = renderer
            .create_material(
                &MaterialDescriptor::new("bumpy", MaterialModel::Pbr).with_normal_map(normal_map),
            )
            .unwrap();
        let glowing = renderer
            .create_material(
                &MaterialDescriptor::new("glowing", MaterialModel::Pbr)
                    .with_emissive(Color::rgb(2.0, 0.5, 0.1)),
            )
            .unwrap();

        renderer.set_camera_position(Vec3::new(0.0, 1.0, 6.0));
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        for material in materials.iter().chain([&bumpy, &glowing]) {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: *material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();

        // 6 materials PBR = 6 bindings distincts, UNE permutation.
        assert_eq!(
            create_pipeline_lines(&journal)
                .iter()
                .filter(|line| line.contains("chaos.material.pbr"))
                .count(),
            1
        );
        assert_eq!(binding_lines(&journal).len(), 6);
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 6);
        assert!(line.ends_with(" cam=(0, 1, 6)"));

        // L'émissif pulse entre deux frames sans recréation.
        let bindings_before = binding_lines(&journal).len();
        renderer
            .set_material_emissive(glowing, Color::rgb(0.5, 0.1, 0.0))
            .unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(binding_lines(&journal).len(), bindings_before);
        assert_eq!(
            renderer.material_info(glowing).unwrap().emissive,
            Color::rgb(0.5, 0.1, 0.0)
        );
    }

    fn env_cubemap(renderer: &mut Renderer, label: &str) -> TextureHandle {
        renderer
            .create_texture(&TextureDescriptor::cube(
                label,
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 24],
            ))
            .unwrap()
    }

    fn environment_lines(journal: &Journal) -> Vec<String> {
        journal
            .entries()
            .into_iter()
            .filter(|entry| entry.starts_with("set_environment"))
            .collect()
    }

    // Le tuple d'un draw ciel dans la ligne render du mock : pas de
    // buffers, 3 sommets générés, pas de binding, matrice identité.
    const SKY_DRAW: &str = ", None, None, 3, b=None, m=[0, 0, 0])";

    #[test]
    fn an_environment_requires_a_living_cubemap() {
        let (mut renderer, _journal) = mock_renderer();
        let flat = small_texture(&mut renderer, "flat");
        let refused = renderer
            .set_environment(&EnvironmentDescriptor::new(flat))
            .unwrap_err();
        assert!(refused.to_string().contains("'flat'"));
        assert!(refused.to_string().contains("D2"));
        assert!(refused.to_string().contains("expects a cubemap"));

        let cube = env_cubemap(&mut renderer, "sky");
        renderer.destroy_texture(cube).unwrap();
        let stale = renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap_err();
        assert!(stale.to_string().contains("stale"));

        let alive = env_cubemap(&mut renderer, "sky2");
        let negative = renderer
            .set_environment(&EnvironmentDescriptor::new(alive).with_intensity(-1.0))
            .unwrap_err();
        assert!(negative.to_string().contains("finite and non-negative"));
        let nan = renderer
            .set_environment(&EnvironmentDescriptor::new(alive).with_intensity(f32::NAN))
            .unwrap_err();
        assert!(nan.to_string().contains("finite and non-negative"));
        assert!(renderer.environment_info().is_none());
    }

    #[test]
    fn setting_the_environment_rebinds_the_backend_once() {
        let (mut renderer, journal) = mock_renderer();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();
        assert_eq!(
            environment_lines(&journal),
            vec![String::from("set_environment index=0")]
        );

        // Le MÊME cubemap re-posé : intensité/ciel mis à jour, zéro rebind.
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube).with_intensity(0.5))
            .unwrap();
        assert_eq!(environment_lines(&journal).len(), 1);
        assert_eq!(renderer.environment_info().unwrap().intensity, 0.5);

        // Un AUTRE cubemap rebinde.
        let other = env_cubemap(&mut renderer, "night");
        renderer
            .set_environment(&EnvironmentDescriptor::new(other))
            .unwrap();
        assert_eq!(
            environment_lines(&journal).pop().unwrap(),
            "set_environment index=1"
        );
    }

    #[test]
    fn clearing_the_environment_rebinds_the_fallback() {
        let (mut renderer, journal) = mock_renderer();
        // Effacer sans environnement : un no-op, aucun appel backend.
        renderer.clear_environment().unwrap();
        assert!(environment_lines(&journal).is_empty());

        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();
        renderer.clear_environment().unwrap();
        renderer.clear_environment().unwrap();
        assert_eq!(
            environment_lines(&journal),
            vec![
                String::from("set_environment index=0"),
                String::from("set_environment none"),
            ]
        );
        assert!(renderer.environment_info().is_none());
    }

    #[test]
    fn the_active_environment_refuses_destruction() {
        let (mut renderer, _journal) = mock_renderer();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();
        let refused = renderer.destroy_texture(cube).unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("'sky' is the active environment: clear it first")
        );
        renderer.clear_environment().unwrap();
        renderer.destroy_texture(cube).unwrap();
    }

    #[test]
    fn the_sky_draw_covers_clear_passes() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        let opaque = plain_material(&mut renderer, "solid");
        let transparent = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();

        for material in [opaque, transparent] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
            renderer
                .queue_draw_to(
                    mirror,
                    DrawCommand {
                        mesh,
                        material,
                        transform: Transform::IDENTITY,
                    },
                )
                .unwrap();
        }
        renderer.render_frame().unwrap();

        // Chaque passe Clear dessine le ciel APRÈS ses opaques et AVANT
        // ses transparents (les draws de scène portent leurs buffers —
        // `Some(…)` — le ciel n'en a pas) ; le rapport compte le draw
        // injecté.
        for line in render_lines(&journal) {
            let sky = line.find(SKY_DRAW).unwrap();
            assert!(line[..sky].contains("Some("));
            assert!(line[sky + SKY_DRAW.len()..].contains("Some("));
        }
        for report in &renderer.frame_report().passes {
            assert_eq!(report.draws, 3);
        }

        // Le rendu immédiat vers une cible efface toujours : ciel inclus.
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap();
        assert!(render_lines(&journal).pop().unwrap().contains(SKY_DRAW));

        // L'environnement effacé : le ciel disparaît de la frame suivante.
        renderer.clear_environment().unwrap();
        renderer.render_frame().unwrap();
        assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
    }

    #[test]
    fn a_keep_pass_never_draws_the_sky() {
        let (mut renderer, journal) = mock_renderer();
        renderer
            .add_pass(
                &surface_pass("overlay")
                    .with_load(PassLoad::Keep)
                    .with_order(1),
            )
            .unwrap();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();
        renderer.render_frame().unwrap();
        let lines = render_lines(&journal);
        // La principale (Clear) reçoit le ciel ; l'overlay (Keep)
        // préserve son image — jamais repeinte par le fond.
        assert!(lines[0].contains(SKY_DRAW));
        assert!(!lines[1].contains(SKY_DRAW));
        assert!(lines[1].ends_with(" load=keep"));
    }

    #[test]
    fn the_sky_respects_its_flag() {
        let (mut renderer, journal) = mock_renderer();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube).with_sky(false))
            .unwrap();
        renderer.render_frame().unwrap();
        // Pas de draw ciel, pas de pipeline ciel — mais l'IBL voyage.
        assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
        assert!(create_pipeline_lines(&journal).is_empty());
        assert_eq!(
            journal
                .entries()
                .iter()
                .filter(|entry| entry.starts_with("environment "))
                .count(),
            1
        );
    }

    #[test]
    fn no_environment_means_no_journal_delta() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_exposure(1.0).unwrap();
        renderer.render_frame().unwrap();
        // Sans environnement et à l'exposition par défaut : aucune ligne
        // nouvelle — le journal historique est intact.
        assert!(journal.entries().iter().all(
            |entry| !entry.starts_with("environment ") && !entry.starts_with("set_environment")
        ));
        assert!(create_pipeline_lines(&journal).is_empty());
    }

    #[test]
    fn the_sky_pipeline_is_one_permutation_per_format() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube))
            .unwrap();
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        let lines = create_pipeline_lines(&journal);
        // Deux formats de destination = deux permutations, créées UNE
        // fois (le cache tient sur les frames suivantes), en LessEqual.
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("create_pipeline chaos.sky.Rgba8UnormSrgb "));
        assert!(lines[1].starts_with("create_pipeline chaos.sky "));
        for line in &lines {
            assert!(line.ends_with(" depth=less_equal"));
        }
    }

    #[test]
    fn the_environment_line_travels_with_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let cube = env_cubemap(&mut renderer, "sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cube).with_intensity(2.0))
            .unwrap();
        renderer.set_exposure(1.5).unwrap();
        renderer.render_frame().unwrap();
        let last_environment = |journal: &Journal| {
            journal
                .entries()
                .iter()
                .rfind(|entry| entry.starts_with("environment "))
                .cloned()
        };
        assert_eq!(
            last_environment(&journal).as_deref(),
            Some("environment intensity=2 exposure=1.5")
        );

        // Sans environnement, l'exposition hors défaut voyage seule.
        renderer.clear_environment().unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(
            last_environment(&journal).as_deref(),
            Some("environment intensity=0 exposure=1.5")
        );
    }

    #[test]
    fn exposure_is_validated_and_persistent() {
        let (mut renderer, _journal) = mock_renderer();
        assert_eq!(renderer.exposure(), 1.0);
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let refused = renderer.set_exposure(invalid).unwrap_err();
            assert!(refused.to_string().contains("positive, finite"));
        }
        renderer.set_exposure(2.0).unwrap();
        renderer.clear_draws();
        assert_eq!(renderer.exposure(), 2.0);
    }

    #[test]
    fn environment_info_reflects_the_state() {
        let (mut renderer, _journal) = mock_renderer();
        let cube = renderer
            .create_texture(
                &TextureDescriptor::cube("hdr", 2, TextureFormat::Rgba16Float, vec![0; 192])
                    .with_mips(TextureMips::Generate),
            )
            .unwrap();
        renderer
            .set_environment(
                &EnvironmentDescriptor::new(cube)
                    .with_intensity(0.8)
                    .with_sky(false),
            )
            .unwrap();
        let info = renderer.environment_info().unwrap();
        assert_eq!(info.label, "hdr");
        assert_eq!(info.intensity, 0.8);
        assert!(!info.sky);
        assert_eq!(info.mip_levels, 2);
        renderer.clear_environment().unwrap();
        assert!(renderer.environment_info().is_none());
    }

    #[test]
    fn the_environment_checkpoint() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "sphere");
        let mut materials = Vec::new();
        for (index, metallic) in [0.0, 1.0].iter().enumerate() {
            materials.push(
                renderer
                    .create_material(
                        &MaterialDescriptor::new(format!("grid.{index}"), MaterialModel::Pbr)
                            .with_metallic(*metallic)
                            .with_roughness(0.2),
                    )
                    .unwrap(),
            );
        }
        // L'environnement HDR mippé : la rugosité IBL parcourt la chaîne.
        let hdr = renderer
            .create_texture(
                &TextureDescriptor::cube("env", 2, TextureFormat::Rgba16Float, vec![0; 192])
                    .with_mips(TextureMips::Generate),
            )
            .unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(hdr))
            .unwrap();

        for material in &materials {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: *material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();

        // Le ciel couvre les deux passes ; métaux et diélectriques
        // partagent la permutation PBR sous le même environnement.
        let lines = render_lines(&journal);
        assert!(lines[0].contains(SKY_DRAW));
        assert!(lines[1].contains(SKY_DRAW));
        assert_eq!(lines[1].matches("m=[").count(), 3);
        assert_eq!(
            create_pipeline_lines(&journal)
                .iter()
                .filter(|line| line.contains("chaos.material.pbr"))
                .count(),
            1
        );
        assert_eq!(
            journal
                .entries()
                .iter()
                .rfind(|entry| entry.starts_with("environment "))
                .map(String::as_str),
            Some("environment intensity=1 exposure=1")
        );

        // L'exposition et l'intensité se règlent SANS rebind ni recréation.
        renderer.set_exposure(2.0).unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(hdr).with_intensity(0.5))
            .unwrap();
        let pipelines_before = create_pipeline_lines(&journal).len();
        renderer.render_frame().unwrap();
        assert_eq!(environment_lines(&journal).len(), 1);
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        assert_eq!(
            journal
                .entries()
                .iter()
                .rfind(|entry| entry.starts_with("environment "))
                .map(String::as_str),
            Some("environment intensity=0.5 exposure=2")
        );

        // Effacé : le ciel disparaît, le fond uni et l'ambiante restent.
        renderer.clear_environment().unwrap();
        renderer.render_frame().unwrap();
        assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
    }

    fn demo_shadow() -> DirectionalShadowDescriptor {
        DirectionalShadowDescriptor::new(ShadowVolume::new(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0)))
    }

    fn lit_caster(renderer: &mut Renderer, label: &str) -> DrawCommand {
        let material = renderer
            .create_material(&MaterialDescriptor::new(label, MaterialModel::Lit))
            .unwrap();
        let mesh = lit_quad_mesh(renderer, label);
        DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        }
    }

    #[test]
    fn configuring_the_shadow_reaches_the_backend_once() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        assert_eq!(
            set_shadow_lines(&journal),
            vec!["set_shadow resolution=2048"]
        );

        // Re-poser la MÊME résolution (biais retouchés) = zéro appel
        // backend — les biais sont des données par frame.
        renderer
            .set_directional_shadow(&demo_shadow().with_depth_bias(0.01).with_normal_bias(0.1))
            .unwrap();
        assert_eq!(set_shadow_lines(&journal).len(), 1);

        // Une autre résolution recrée la map.
        renderer
            .set_directional_shadow(&demo_shadow().with_resolution(1024))
            .unwrap();
        assert_eq!(
            set_shadow_lines(&journal),
            vec!["set_shadow resolution=2048", "set_shadow resolution=1024"]
        );

        // Effacer libère — et l'effacement répété est un no-op.
        renderer.clear_directional_shadow().unwrap();
        renderer.clear_directional_shadow().unwrap();
        assert_eq!(
            set_shadow_lines(&journal),
            vec![
                "set_shadow resolution=2048",
                "set_shadow resolution=1024",
                "set_shadow none"
            ]
        );
    }

    #[test]
    fn shadow_settings_are_validated_before_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        let refused = renderer
            .set_directional_shadow(&demo_shadow().with_resolution(8))
            .unwrap_err();
        assert!(refused.to_string().contains("16..=8192"));
        let flat = renderer
            .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::ZERO,
                Vec3::new(10.0, 0.0, 10.0),
            )))
            .unwrap_err();
        assert!(flat.to_string().contains("half extents"));
        assert!(set_shadow_lines(&journal).is_empty());
        assert!(renderer.directional_shadow_info().is_none());
    }

    #[test]
    fn shadow_info_mirrors_the_state() {
        let (mut renderer, _journal) = mock_renderer();
        assert!(renderer.directional_shadow_info().is_none());
        renderer
            .set_directional_shadow(&demo_shadow().with_resolution(512).with_depth_bias(0.01))
            .unwrap();
        let info = renderer.directional_shadow_info().unwrap();
        assert_eq!(info.resolution, 512);
        assert_eq!(info.depth_bias, 0.01);
        assert_eq!(info.volume.half_extents, Vec3::new(10.0, 10.0, 10.0));
        renderer.clear_directional_shadow().unwrap();
        assert!(renderer.directional_shadow_info().is_none());
    }

    #[test]
    fn the_shadow_pass_travels_first_with_the_casting_draws() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let command = lit_caster(&mut renderer, "caster");
        renderer.queue_draw(command);
        renderer.render_frame().unwrap();

        // Le pipeline d'ombre est une permutation depth-only dédiée,
        // étiquetée par le stride de son layout (LitVertex = 32).
        assert!(
            create_pipeline_lines(&journal)
                .iter()
                .any(|line| line.starts_with("create_pipeline chaos.shadow.32 ")
                    && line.contains(" depth_only"))
        );
        // La ligne shadow PRÉCÈDE la ligne render de la passe principale,
        // porte le caster (binding None) et la résolution.
        let entries = journal.entries();
        let shadow_at = entries
            .iter()
            .position(|entry| entry.starts_with("shadow "))
            .unwrap();
        let render_at = entries
            .iter()
            .position(|entry| entry.starts_with("render "))
            .unwrap();
        assert!(shadow_at < render_at);
        let shadow = &entries[shadow_at];
        assert!(shadow.contains("res=2048"));
        assert!(shadow.contains("light=0"));
        assert!(shadow.contains("b=None"));
        // Le rapport dédié reflète la passe.
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );
    }

    #[test]
    fn without_a_directional_light_the_shadow_pass_is_absent() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        let command = lit_caster(&mut renderer, "caster");
        renderer.queue_draw(command);

        // Aucune lumière : pas de passe d'ombre, rien de fatal.
        renderer.render_frame().unwrap();
        assert!(shadow_lines(&journal).is_empty());
        assert_eq!(renderer.frame_report().shadow, None);

        // Une ponctuelle seule ne projette pas en V1.
        renderer.submit_light(Light::point(Vec3::Y, Color::WHITE, 1.0, 5.0));
        renderer.render_frame().unwrap();
        assert!(shadow_lines(&journal).is_empty());

        // Une directionnelle DÉSACTIVÉE est filtrée de la collection —
        // elle ne projette pas non plus.
        let mut sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0);
        sun.set_enabled(false);
        renderer.submit_light(sun);
        renderer.render_frame().unwrap();
        assert!(shadow_lines(&journal).is_empty());
        assert_eq!(renderer.frame_report().shadow, None);
    }

    #[test]
    fn the_first_enabled_directional_is_the_caster() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        let command = lit_caster(&mut renderer, "caster");
        renderer.queue_draw(command);
        // Une ponctuelle soumise AVANT : la directionnelle est à
        // l'index 1 de la collection — l'index voyage au shader.
        renderer.submit_light(Light::point(Vec3::Y, Color::WHITE, 1.0, 5.0));
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        renderer.submit_light(Light::directional(Vec3::NEG_X, Color::WHITE, 0.5));
        renderer.render_frame().unwrap();
        let shadow = shadow_lines(&journal).pop().unwrap();
        assert!(shadow.contains("light=1"));
    }

    #[test]
    fn transparent_and_non_casting_materials_never_cast() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let caster = lit_caster(&mut renderer, "caster");
        renderer.queue_draw(caster);
        let opted_out = renderer
            .create_material(
                &MaterialDescriptor::new("shy", MaterialModel::Lit).without_shadow_cast(),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "extra");
        renderer.queue_draw(DrawCommand {
            mesh,
            material: opted_out,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        // Trois draws dans la passe, UN SEUL caster dans l'ombre.
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );
        let shadow = shadow_lines(&journal).pop().unwrap();
        assert_eq!(shadow.matches("m=[").count(), 1);
    }

    #[test]
    fn the_sky_never_casts_a_shadow() {
        let (mut renderer, journal) = mock_renderer();
        let cubemap = renderer
            .create_texture(&TextureDescriptor::cube(
                "sky.cube",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 4 * 6],
            ))
            .unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let command = lit_caster(&mut renderer, "caster");
        renderer.queue_draw(command);
        renderer.render_frame().unwrap();
        // La passe principale porte le caster ET le ciel injecté ; la
        // passe d'ombre ne porte QUE le caster.
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );
        let render = render_lines(&journal).pop().unwrap();
        assert_eq!(render.matches("m=[").count(), 2);
    }

    #[test]
    fn shadow_pipelines_are_permutations_cached_by_layout_and_state() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let first = lit_caster(&mut renderer, "first");
        let second = lit_caster(&mut renderer, "second");
        let leafy = renderer
            .create_material(&MaterialDescriptor::new("leafy", MaterialModel::Lit).double_sided())
            .unwrap();
        renderer.queue_draw(first);
        renderer.queue_draw(second);
        renderer.queue_draw(DrawCommand {
            mesh: first.mesh,
            material: leafy,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        renderer.queue_draw(first);
        renderer.render_frame().unwrap();
        // UNE permutation par (layout, culling, instanced) — stable
        // entre frames : deux materials Lit partagent la même, le
        // double-sided a la sienne, et le DOUBLON de `first` (la file
        // persiste entre les frames) fusionne en caster instancié à la
        // deuxième frame — sa permutation `.instanced` est la troisième
        // et dernière. Le label porte le stride du layout (LitVertex =
        // 32).
        let shadow_pipelines: Vec<String> = create_pipeline_lines(&journal)
            .into_iter()
            .filter(|line| line.contains("chaos.shadow"))
            .collect();
        assert_eq!(shadow_pipelines.len(), 3);
        assert!(shadow_pipelines[0].starts_with("create_pipeline chaos.shadow.32 "));
        assert!(shadow_pipelines[1].starts_with("create_pipeline chaos.shadow.32.double_sided "));
        assert!(shadow_pipelines[2].starts_with("create_pipeline chaos.shadow.32.instanced "));
        assert!(shadow_lines(&journal).pop().unwrap().contains("inst=2"));
    }

    #[test]
    fn a_layout_without_position_is_excluded_from_casting() {
        let (mut renderer, journal) = mock_renderer();
        let layout = VertexLayout::packed(&[VertexAttributeFormat::Float32x2]);
        let mut context = PipelineContext {
            pipeline_cache: &mut renderer.pipeline_cache,
            sky_pipelines: &mut renderer.sky_pipelines,
            shadow_pipelines: &mut renderer.shadow_pipelines,
            instanced_pipelines: &mut renderer.instanced_pipelines,
            debug_pipelines: &mut renderer.debug_pipelines,
            backend: renderer.backend.as_mut(),
            shaders: &renderer.shaders,
            lifetime: &mut renderer.lifetime,
        };
        assert!(Renderer::resolve_shadow_pipeline(&mut context, &layout, false, false).is_none());
        // Mémoïsé : le second appel ne retente rien, aucun pipeline créé.
        assert!(Renderer::resolve_shadow_pipeline(&mut context, &layout, false, false).is_none());
        assert!(
            create_pipeline_lines(&journal)
                .iter()
                .all(|line| !line.contains("chaos.shadow"))
        );
    }

    #[test]
    fn render_to_target_carries_no_shadow_pass() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let target = renderer
            .create_render_target(&RenderTargetDescriptor::new(
                "vignette",
                64,
                64,
                TextureFormat::Rgba8UnormSrgb,
            ))
            .unwrap();
        let command = lit_caster(&mut renderer, "caster");
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command])
            .unwrap();
        // Le chemin immédiat ne rend pas d'ombre (il échantillonne la
        // map du dernier plan) et ne touche pas le rapport.
        assert!(shadow_lines(&journal).is_empty());
        assert_eq!(renderer.frame_report().shadow, None);
    }

    #[test]
    fn an_empty_plan_skips_the_shadow_too() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let main = renderer.main_pass();
        renderer.set_pass_enabled(main, false).unwrap();
        renderer.render_frame().unwrap();
        assert!(shadow_lines(&journal).is_empty());
        assert_eq!(renderer.frame_report().shadow, None);
    }

    #[test]
    fn shadow_maps_are_counted_in_the_stats() {
        let (mut renderer, _journal) = mock_renderer();
        let baseline = renderer.resource_stats();
        assert_eq!(baseline.shadow_maps, KindStats::default());
        renderer
            .set_directional_shadow(&demo_shadow().with_resolution(1024))
            .unwrap();
        let stats = renderer.resource_stats();
        assert_eq!(stats.shadow_maps.alive, 1);
        assert_eq!(stats.shadow_maps.bytes, 1024 * 1024 * 4);
        assert_eq!(
            stats.estimated_bytes,
            baseline.estimated_bytes + 1024 * 1024 * 4
        );
        renderer.clear_directional_shadow().unwrap();
        assert_eq!(renderer.resource_stats(), baseline);
    }

    #[test]
    fn transparents_are_never_instanced() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for z in [-1.0, -2.0, -3.0] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        renderer.render_frame().unwrap();
        // Trois draws individuels, triés par profondeur — jamais un
        // `inst=` : le tri des transparents prime sur le regroupement.
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 3);
        assert!(!line.contains("inst="));
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].draws, 3);
        assert_eq!(report.passes[0].draw_calls, 3);
    }

    #[test]
    fn masked_and_opaque_runs_never_share_a_batch() {
        let (mut renderer, journal) = mock_renderer();
        let solid = renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for material in [grid, solid, grid, solid] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();
        // DEUX batches de 2 : l'opaque sur sa permutation `.instanced`,
        // le masked sur `.masked.instanced` (entrées vs_instanced +
        // fs_masked) — les catégories ne se mélangent jamais.
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("inst=2").count(), 2);
        let instanced_pipelines: Vec<String> = create_pipeline_lines(&journal)
            .into_iter()
            .filter(|line| line.contains(" instanced"))
            .collect();
        assert_eq!(instanced_pipelines.len(), 2);
        assert!(
            instanced_pipelines
                .iter()
                .any(|line| line.starts_with("create_pipeline chaos.material.lit.instanced "))
        );
        assert!(instanced_pipelines.iter().any(|line| {
            line.starts_with("create_pipeline chaos.material.lit.masked.instanced ")
                && line.contains(" entry=fs_masked")
        }));
        assert_eq!(
            renderer.frame_report().passes[0].breakdown,
            DrawBreakdown {
                opaque: 2,
                masked: 2,
                transparent: 0,
                injected: 0,
            }
        );
        assert_eq!(renderer.frame_report().passes[0].draws, 4);
        assert_eq!(renderer.frame_report().passes[0].draw_calls, 2);
    }

    #[test]
    fn render_to_target_batches_too() {
        let (mut renderer, journal) = mock_renderer();
        let solid = renderer
            .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        let target = renderer
            .create_render_target(&RenderTargetDescriptor::new(
                "vignette",
                64,
                64,
                TextureFormat::Rgba8UnormSrgb,
            ))
            .unwrap();
        let command = DrawCommand {
            mesh,
            material: solid,
            transform: Transform::IDENTITY,
        };
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command, command])
            .unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
        assert!(line.contains("inst=2"));
        // La permutation instanciée VISE le format de la cible — le
        // descripteur, pas seulement le label (le run GPU l'exige).
        assert!(create_pipeline_lines(&journal).iter().any(|line| {
            line.contains(".instanced") && line.contains(" target=Rgba8UnormSrgb")
        }));
    }

    #[test]
    fn instanced_shadow_casters_report_their_draw_calls() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let caster = lit_caster(&mut renderer, "caster");
        for x in 0u8..5 {
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(f32::from(x), 0.0, 0.0)),
                ..caster
            });
        }
        renderer.render_frame().unwrap();
        // Cinq casters d'un même (material, mesh) = UN draw d'ombre
        // instancié — le rapport dit les objets ET les soumissions.
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 5,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );
        let shadow = shadow_lines(&journal).pop().unwrap();
        assert_eq!(shadow.matches("m=[").count(), 1);
        assert!(shadow.contains("inst=5"));
    }

    #[test]
    fn checkpoint_instancing_v1_a_crowd_collapses_to_a_few_draw_calls() {
        // LE checkpoint Instancing V1 : 500 objets compatibles + des
        // incompatibles mélangés — les draw calls tombent d'un ordre de
        // grandeur, les incompatibles restent des draws classiques, les
        // ombres profitent pareil, et le consommateur n'a rien changé
        // (il soumet toujours objet par objet).
        let (mut renderer, journal) = mock_renderer();
        // Le volume d'ombre couvre TOUTE la foule (x 0..499) : le
        // culling d'ombre ne rejette rien ici — il a son propre test.
        renderer
            .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::new(250.0, 0.0, 0.0),
                Vec3::new(300.0, 20.0, 300.0),
            )))
            .unwrap();
        let crowd = lit_caster(&mut renderer, "crowd");
        let loner = lit_caster(&mut renderer, "loner");
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let queue_scene = |renderer: &mut Renderer| {
            renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
            for index in 0..500u32 {
                renderer.queue_draw(DrawCommand {
                    transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                    ..crowd
                });
            }
            renderer.queue_draw(loner);
            for z in [-1.0, -2.0, -3.0] {
                renderer.queue_draw(DrawCommand {
                    mesh: loner.mesh,
                    material: glass,
                    transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
                });
            }
        };

        // FRAME 1 : 504 objets logiques → 5 soumissions (la foule en 1,
        // le solitaire en 1, les 3 transparents individuels).
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].draws, 504);
        assert_eq!(report.passes[0].draw_calls, 5);
        // Les ombres profitent pareil : 501 casters (les transparents
        // jamais) → 2 soumissions.
        assert_eq!(
            report.shadow,
            Some(ShadowReport {
                draws: 501,
                draw_calls: 2,
                culled: 0,
                resolution: 2048
            })
        );
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains("inst=500"));

        // FRAME 2 : mêmes soumissions — AUCUN pipeline de plus (le
        // cache des permutations instanciées tient), mêmes comptes.
        let pipelines_before = create_pipeline_lines(&journal).len();
        renderer.clear_draws();
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        assert_eq!(renderer.frame_report().passes[0].draw_calls, 5);
    }

    #[test]
    fn checkpoint_transparency_v1_full_scene_over_two_frames() {
        // LE checkpoint Transparency & Ordering V1 : les trois
        // catégories sous ombres et ciel, l'ordre à quatre temps, le
        // tri qui SUIT la caméra entre deux frames, le cutoff retouché
        // à chaud, la ventilation exacte.
        let (mut renderer, journal) = mock_renderer();
        let cubemap = renderer
            .create_texture(&TextureDescriptor::cube(
                "chk.sky",
                1,
                TextureFormat::Rgba8Unorm,
                vec![0; 4 * 6],
            ))
            .unwrap();
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        let solid = renderer
            .create_material(&MaterialDescriptor::new("chk.solid", MaterialModel::Lit))
            .unwrap();
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("chk.grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked)
                    .with_alpha_cutoff(0.4),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("chk.glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "chk.pane");
        let sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9);
        let queue_scene = |renderer: &mut Renderer| {
            for (material, z) in [(solid, -4.0), (grid, -2.0), (glass, -1.0), (glass, -6.0)] {
                renderer.queue_draw(DrawCommand {
                    mesh,
                    material,
                    transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
                });
            }
        };

        // FRAME 1 — caméra devant : le verre LOINTAIN (-6) d'abord.
        renderer.set_camera_position(Vec3::new(0.0, 0.0, 2.0));
        renderer.submit_light(sun.clone());
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let report = renderer.frame_report();
        assert_eq!(
            report.passes[0].breakdown,
            DrawBreakdown {
                opaque: 1,
                masked: 1,
                transparent: 2,
                injected: 1,
            }
        );
        // Opaque et masked projettent (2 casters), le transparent non.
        assert_eq!(
            report.shadow,
            Some(ShadowReport {
                draws: 2,
                draw_calls: 2,
                culled: 0,
                resolution: 2048
            })
        );
        let first = render_lines(&journal).pop().unwrap();
        assert!(first.find("m=[0, 0, -6]").unwrap() > first.find("m=[0, 0, -2]").unwrap());
        assert!(first.find("m=[0, 0, -6]").unwrap() < first.find("m=[0, 0, -1]").unwrap());

        // FRAME 2 — caméra DERRIÈRE la scène : l'ordre des verres
        // s'inverse ; le cutoff est retouché À CHAUD (in-place, aucune
        // recréation).
        let pipelines_before = create_pipeline_lines(&journal).len();
        renderer.set_material_alpha_cutoff(grid, 0.8).unwrap();
        renderer.clear_draws();
        renderer.set_camera_position(Vec3::new(0.0, 0.0, -12.0));
        renderer.submit_light(sun);
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let second = render_lines(&journal).pop().unwrap();
        assert!(second.find("m=[0, 0, -1]").unwrap() < second.find("m=[0, 0, -6]").unwrap());
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        assert_eq!(renderer.material_info(grid).unwrap().alpha_cutoff, 0.8);
        assert_eq!(
            renderer.frame_report().passes[0].breakdown,
            DrawBreakdown {
                opaque: 1,
                masked: 1,
                transparent: 2,
                injected: 1,
            }
        );
    }

    #[test]
    fn checkpoint_shadows_v1_full_scene_over_two_frames() {
        // LE checkpoint Shadows V1 : une scène complète (caster,
        // non-caster, transparent, receive-off) sous réglages d'ombre,
        // le volume retouché entre deux frames SANS recréation backend,
        // le toggle du soleil observable, l'effacement revenant au
        // niveau de base.
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.set_ambient_light(Color::WHITE, 0.05);

        let caster = lit_caster(&mut renderer, "chk.caster");
        let shy = renderer
            .create_material(
                &MaterialDescriptor::new("chk.shy", MaterialModel::Lit).without_shadow_cast(),
            )
            .unwrap();
        let numb = renderer
            .create_material(
                &MaterialDescriptor::new("chk.numb", MaterialModel::Pbr).without_shadow_receive(),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("chk.glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "chk.mesh");
        let mut sun = Light::directional(Vec3::new(-1.0, -2.0, -1.0), Color::WHITE, 0.9);

        // FRAME 1 : quatre draws, DEUX casters (le caster + le numb —
        // receive-off projette quand même), la passe d'ombre en tête.
        renderer.submit_light(sun.clone());
        renderer.queue_draw(caster);
        for material in [shy, numb, glass] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
        }
        renderer.render_frame().unwrap();
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 2,
                draw_calls: 2,
                culled: 0,
                resolution: 2048
            })
        );
        let first_shadow = shadow_lines(&journal).pop().unwrap();

        // FRAME 2 : le volume et les biais retouchés À CHAUD — zéro
        // set_shadow backend de plus, la vue de lumière change.
        renderer
            .set_directional_shadow(
                &DirectionalShadowDescriptor::new(ShadowVolume::new(
                    Vec3::new(2.0, 0.0, 2.0),
                    Vec3::new(6.0, 6.0, 6.0),
                ))
                .with_depth_bias(0.004),
            )
            .unwrap();
        renderer.clear_draws();
        renderer.submit_light(sun.clone());
        renderer.queue_draw(caster);
        renderer.render_frame().unwrap();
        assert_eq!(set_shadow_lines(&journal).len(), 1);
        let second_shadow = shadow_lines(&journal).pop().unwrap();
        assert_ne!(first_shadow, second_shadow);
        assert_eq!(
            renderer.frame_report().shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 0,
                resolution: 2048
            })
        );

        // FRAME 3 : le soleil coupé — la passe d'ombre disparaît, la
        // scène continue.
        sun.set_enabled(false);
        renderer.clear_draws();
        renderer.submit_light(sun);
        renderer.queue_draw(caster);
        renderer.render_frame().unwrap();
        assert_eq!(shadow_lines(&journal).len(), 2);
        assert_eq!(renderer.frame_report().shadow, None);

        // Effacement : la map libérée, le rapport et les stats au
        // niveau de base.
        renderer.clear_directional_shadow().unwrap();
        assert_eq!(
            set_shadow_lines(&journal).last().map(String::as_str),
            Some("set_shadow none")
        );
        assert_eq!(renderer.resource_stats().shadow_maps, KindStats::default());
        assert!(renderer.directional_shadow_info().is_none());
    }

    #[test]
    fn the_mock_reports_deterministic_capabilities() {
        let (renderer, journal) = mock_renderer();
        let capabilities = renderer.capabilities();
        assert_eq!(capabilities.backend, "mock");
        assert_eq!(capabilities.adapter, "journal");
        assert_eq!(
            capabilities.limits,
            crate::capabilities::DeviceLimits::default()
        );
        // AUCUNE feature optionnelle n'est supposée : les timestamps
        // sont COUPÉS avec leur raison — et rien n'empêche de rendre.
        assert!(matches!(
            &capabilities.decision("timestamp queries").unwrap().status,
            crate::capabilities::CapabilityStatus::Disabled { reason } if !reason.is_empty()
        ));
        // La consultation n'écrit RIEN au journal.
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn device_limits_refuse_oversized_resources_by_name() {
        let (mut renderer, _journal) = mock_renderer();
        // La texture au-delà de la limite device : refusée en NOMMANT
        // la valeur ET la limite — jamais une erreur backend.
        let refused = renderer
            .create_texture(&TextureDescriptor::sampled(
                "huge",
                8193,
                1,
                TextureFormat::R8Unorm,
                vec![0; 8193],
            ))
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("8193x1 exceeds the device texture limit (8192)")
        );
        let refused = renderer
            .create_render_target(&RenderTargetDescriptor::new(
                "huge",
                8193,
                1,
                TextureFormat::Rgba8UnormSrgb,
            ))
            .unwrap_err();
        assert!(refused.to_string().contains("device texture limit (8192)"));
        // À la limite EXACTE : accepté.
        assert!(
            renderer
                .create_texture(&TextureDescriptor::sampled(
                    "edge",
                    8192,
                    1,
                    TextureFormat::R8Unorm,
                    vec![0; 8192],
                ))
                .is_ok()
        );
    }

    #[test]
    fn lowered_device_limits_speak_for_the_device() {
        // Un device plus petit que l'engine : chaque refus parle au nom
        // du DEVICE — distinct des bornes engine.
        let limits = crate::capabilities::DeviceLimits {
            max_texture_2d: 1024,
            max_buffer_bytes: 1000,
            max_anisotropy: 8,
            ..crate::capabilities::DeviceLimits::default()
        };
        let (mut renderer, _journal) = mock_renderer_with_limits(limits);
        // Texture 2048 : légale pour l'engine, refusée par CE device.
        let refused = renderer
            .create_texture(&TextureDescriptor::sampled(
                "big",
                2048,
                1,
                TextureFormat::R8Unorm,
                vec![0; 2048],
            ))
            .unwrap_err();
        assert!(refused.to_string().contains("device texture limit (1024)"));
        // Buffer 1001 octets : refusé par les DEUX chemins — le public
        // et celui des meshes.
        let refused = renderer
            .create_buffer(&BufferDescriptor::vertex("big", vec![0; 1001]))
            .unwrap_err();
        assert!(refused.to_string().contains("device buffer limit (1000)"));
        let sphere = LitGeometry::sphere([0.0, 0.0, 0.0], 1.0, 16, 16);
        let refused = renderer.create_lit_mesh("ball", &sphere).unwrap_err();
        assert!(refused.to_string().contains("device buffer limit (1000)"));
        // L'ombre 2048 : le descripteur la VALIDE (16..=8192), le
        // device la refuse — le message nomme la borne DEVICE.
        let descriptor = DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::ZERO,
            Vec3::new(10.0, 10.0, 10.0),
        ));
        let refused = renderer.set_directional_shadow(&descriptor).unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("shadow map resolution 2048 exceeds the device texture limit (1024)")
        );
        assert!(
            renderer
                .set_directional_shadow(&descriptor.with_resolution(1024))
                .is_ok()
        );
        // L'anisotropie x16 : légale pour le descripteur (tout-Linear
        // respecté), refusée par CE device (plafond x8).
        let refused = renderer
            .create_sampler(
                &SamplerDescriptor::new("aniso")
                    .with_mip_filter(SamplerFilter::Linear)
                    .with_anisotropy(16),
            )
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("anisotropy x16 exceeds the device ceiling (x8)")
        );
        assert!(
            renderer
                .create_sampler(
                    &SamplerDescriptor::new("aniso.ok")
                        .with_mip_filter(SamplerFilter::Linear)
                        .with_anisotropy(8),
                )
                .is_ok()
        );
        // À la limite exacte : accepté.
        assert!(
            renderer
                .create_texture(&TextureDescriptor::sampled(
                    "edge",
                    1024,
                    1,
                    TextureFormat::R8Unorm,
                    vec![0; 1024],
                ))
                .is_ok()
        );
    }

    #[test]
    fn checkpoint_robustness_v1_no_capability_is_implicit() {
        // LE checkpoint Robustesse V1 : le rapport COMPLET (chaque
        // domaine expliqué), les configurations impossibles refusées en
        // nommant, une feature optionnelle absente n'empêche RIEN, et
        // la consultation ne trouble jamais le journal.
        let (mut renderer, journal) = mock_renderer();
        let capabilities = renderer.capabilities().clone();
        assert!(!capabilities.backend.is_empty());
        assert!(!capabilities.adapter.is_empty());
        for decision in &capabilities.decisions {
            assert!(!decision.domain.is_empty());
            assert!(!decision.detail.is_empty());
            if let crate::capabilities::CapabilityStatus::Disabled { reason }
            | crate::capabilities::CapabilityStatus::Fallback { reason } = &decision.status
            {
                assert!(
                    !reason.is_empty(),
                    "{} must explain itself",
                    decision.domain
                );
            }
        }
        // Le Display est la lecture sans UI.
        let text = capabilities.to_string();
        assert!(text.contains("capabilities: mock on journal"));
        assert!(text.contains("timestamp queries: disabled"));
        // Les timestamps COUPÉS n'empêchent rien : la frame rend, le
        // temps GPU est dit indisponible.
        let material = plain_material(&mut renderer, "prop");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().frame.resolved, 1);
        assert!(matches!(
            &renderer.diagnostics().gpu,
            crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
        ));
        // Une configuration IMPOSSIBLE échoue proprement, le registre
        // intact : la même scène rend encore.
        assert!(
            renderer
                .create_texture(&TextureDescriptor::sampled(
                    "huge",
                    9000,
                    9000,
                    TextureFormat::R8Unorm,
                    vec![0; 81_000_000],
                ))
                .is_err()
        );
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().frame.resolved, 1);
        // Le journal n'a jamais vu passer ni le rapport ni le refus.
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !entry.contains("huge") && !entry.contains("capabilities"))
        );
    }

    #[test]
    fn the_mock_backend_declares_its_gpu_time_unavailable() {
        // « Aucune valeur ne doit être inventée » : un backend qui ne
        // mesure pas le DIT — la raison est nommée, jamais un zéro.
        let (renderer, _journal) = mock_renderer();
        assert!(matches!(
            renderer.backend.gpu_frame_time(),
            crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
        ));
        // Avant la première frame : le snapshot par défaut, honnête.
        assert!(matches!(
            &renderer.diagnostics().gpu,
            crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
        ));
    }

    #[test]
    fn the_cpu_budget_is_validated_and_stored() {
        let (mut renderer, _journal) = mock_renderer();
        assert_eq!(renderer.diagnostics().budget.budget_ms, None);
        renderer.set_cpu_budget(Some(4.0));
        assert_eq!(renderer.diagnostics().budget.budget_ms, Some(4.0));
        renderer.set_cpu_budget(Some(f32::NAN));
        renderer.set_cpu_budget(Some(-1.0));
        renderer.set_cpu_budget(Some(0.0));
        assert_eq!(renderer.diagnostics().budget.budget_ms, Some(4.0));
        renderer.set_cpu_budget(None);
        assert_eq!(renderer.diagnostics().budget.budget_ms, None);
    }

    #[test]
    fn diagnostics_count_the_instanced_crowd_exactly() {
        // La scène de l'instancing : 500 compatibles + 1 solitaire +
        // 3 transparents — chaque compteur du snapshot est EXACT.
        let (mut renderer, _journal) = mock_renderer();
        let crowd = lit_caster(&mut renderer, "crowd");
        let loner = lit_caster(&mut renderer, "loner");
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        for index in 0..500u32 {
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                ..crowd
            });
        }
        renderer.queue_draw(loner);
        for z in [-1.0, -2.0, -3.0] {
            renderer.queue_draw(DrawCommand {
                mesh: loner.mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        renderer.render_frame().unwrap();
        let frame = renderer.diagnostics().frame;
        assert_eq!(frame.submitted, 504);
        assert_eq!(frame.resolved, 504);
        assert_eq!(frame.classic_draws, 4);
        assert_eq!(frame.instanced_draws, 1);
        assert_eq!(frame.instances, 500);
        assert_eq!(frame.culled, 0);
        assert_eq!(frame.injected, 0);
        // Le quad éclairé fait 6 indices = 2 triangles : la foule en
        // 1000, le solitaire 2, les verres 6.
        assert_eq!(frame.triangles, 1008);
        // Trois pipelines se succèdent (instancié → classique →
        // transparent), trois bindings distincts (les verres partagent).
        assert_eq!(frame.pipeline_switches, 3);
        assert_eq!(frame.material_switches, 3);
        assert_eq!(frame.passes_executed, 1);
        let pass = &renderer.diagnostics().passes[0];
        assert_eq!(pass.draw_calls, 5);
        assert_eq!(pass.instances, 500);
        assert!(pass.resolve_cpu_ms.is_finite() && pass.resolve_cpu_ms >= 0.0);
        // Le coût CPU est MESURÉ, le GPU du mock est indisponible DIT.
        let cpu = renderer.diagnostics().cpu;
        assert!(cpu.total_ms >= cpu.backend_ms);
        assert!(matches!(
            &renderer.diagnostics().gpu,
            crate::diagnostics::GpuTiming::Unavailable { reason }
                if reason.contains("mock")
        ));
    }

    #[test]
    fn diagnostics_measure_the_culling_gains() {
        let (mut renderer, _journal) = mock_renderer();
        let material = plain_material(&mut renderer, "crowd");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        for index in 0..10u32 {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(250.0 * index as f32, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        // Cinq visibles fusionnés, cinq cullés : le gain se LIT — et
        // les triangles ne comptent QUE les visibles (12 par cube).
        let frame = renderer.diagnostics().frame;
        assert_eq!(frame.submitted, 10);
        assert_eq!(frame.resolved, 5);
        assert_eq!(frame.culled, 5);
        assert_eq!(frame.instanced_draws, 1);
        assert_eq!(frame.instances, 5);
        assert_eq!(frame.triangles, 60);
    }

    #[test]
    fn the_sky_and_debug_are_counted_honestly() {
        let (mut renderer, _journal) = mock_renderer();
        let cubemap = env_cubemap(&mut renderer, "diag.sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
        renderer.render_frame().unwrap();
        let frame = renderer.diagnostics().frame;
        // Le ciel = 1 injecté et 1 TRIANGLE ; le debug = 2 injectés et
        // 4 SEGMENTS (la ligne + la croix à trois axes) — jamais des
        // triangles.
        assert_eq!(frame.resolved, 3);
        assert_eq!(frame.injected, 3);
        assert_eq!(frame.triangles, 1);
        assert_eq!(frame.debug_segments, 4);
        assert_eq!(frame.classic_draws, 1);
        assert_eq!(frame.pipeline_switches, 3);
        assert_eq!(frame.material_switches, 0);
    }

    #[test]
    fn surface_events_accumulate_by_reason() {
        for (outcome, check) in [
            (
                FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable),
                0usize,
            ),
            (
                FrameOutcome::Skipped(FrameSkipReason::SurfaceReconfigured),
                1,
            ),
            (FrameOutcome::Skipped(FrameSkipReason::ZeroArea), 2),
            (FrameOutcome::Rendered, 3),
        ] {
            let (mut renderer, _journal) = mock_renderer_with(outcome);
            renderer.render_frame().unwrap();
            renderer.render_frame().unwrap();
            let surface = renderer.diagnostics().surface;
            let counters = [
                surface.skipped_unavailable,
                surface.reconfigured,
                surface.zero_area,
                surface.presented,
            ];
            for (index, counter) in counters.iter().enumerate() {
                assert_eq!(*counter, if index == check { 2 } else { 0 });
            }
        }
    }

    #[test]
    fn the_budget_counts_overruns() {
        let (mut renderer, _journal) = mock_renderer();
        // Sans budget : jamais de dépassement.
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().budget.over_budget_frames, 0);
        assert!(!renderer.diagnostics().budget.last_frame_over);
        // Un budget minuscule : chaque frame dépasse, le cumul avance.
        renderer.set_cpu_budget(Some(f32::MIN_POSITIVE));
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().budget.over_budget_frames, 2);
        assert!(renderer.diagnostics().budget.last_frame_over);
        // Le budget retiré : le cumul reste, le présent redevient sain.
        renderer.set_cpu_budget(None);
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().budget.over_budget_frames, 2);
        assert!(!renderer.diagnostics().budget.last_frame_over);
    }

    #[test]
    fn degraded_permutations_and_builtins_are_visible_fallbacks() {
        let (mut renderer, _journal) = mock_renderer();
        // Un material Unlit sans texture consomme les fallbacks builtin.
        let material = renderer
            .create_material(&MaterialDescriptor::new("bare", MaterialModel::Unlit))
            .unwrap();
        let mesh = renderer
            .create_textured_mesh(
                "pane",
                &TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0),
            )
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        // Une permutation d'ombre IMPOSSIBLE (layout sans position) est
        // mémoïsée — un chemin dégradé VISIBLE au snapshot.
        let layout = VertexLayout::packed(&[VertexAttributeFormat::Float32x2]);
        {
            let mut context = PipelineContext {
                pipeline_cache: &mut renderer.pipeline_cache,
                sky_pipelines: &mut renderer.sky_pipelines,
                shadow_pipelines: &mut renderer.shadow_pipelines,
                instanced_pipelines: &mut renderer.instanced_pipelines,
                debug_pipelines: &mut renderer.debug_pipelines,
                backend: renderer.backend.as_mut(),
                shaders: &renderer.shaders,
                lifetime: &mut renderer.lifetime,
            };
            assert!(
                Renderer::resolve_shadow_pipeline(&mut context, &layout, false, false).is_none()
            );
        }
        renderer.render_frame().unwrap();
        let fallbacks = renderer.diagnostics().fallbacks;
        assert_eq!(fallbacks.degraded_permutations, 1);
        assert!(fallbacks.fallback_textures >= 1);
        assert!(fallbacks.fallback_samplers >= 1);
    }

    #[test]
    fn checkpoint_diagnostics_v1_the_frame_explains_itself() {
        // LE checkpoint Diagnostics V1 : la scène composée — foule
        // instanciée, hors-champ cullés, masked, transparents, ciel,
        // ombre, debug sous les deux profondeurs — et CHAQUE champ du
        // snapshot exact, stable sur deux frames, les gains de
        // l'instancing et du culling MESURABLES, le GPU indisponible
        // DIT, les ressources cohérentes.
        let (mut renderer, _journal) = mock_renderer();
        renderer
            .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::new(100.0, 0.0, 0.0),
                Vec3::new(150.0, 20.0, 150.0),
            )))
            .unwrap();
        let cubemap = env_cubemap(&mut renderer, "chk.sky");
        renderer
            .set_environment(&EnvironmentDescriptor::new(cubemap))
            .unwrap();
        let crowd = lit_caster(&mut renderer, "crowd");
        let grid = renderer
            .create_material(
                &MaterialDescriptor::new("grid", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Masked),
            )
            .unwrap();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let queue_scene = |renderer: &mut Renderer| {
            renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
            for index in 0..200u32 {
                renderer.queue_draw(DrawCommand {
                    transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                    ..crowd
                });
            }
            for index in 0..5u32 {
                renderer.queue_draw(DrawCommand {
                    transform: Transform::from_translation(Vec3::new(
                        5000.0 + index as f32,
                        0.0,
                        0.0,
                    )),
                    ..crowd
                });
            }
            renderer.queue_draw(DrawCommand {
                mesh: crowd.mesh,
                material: grid,
                transform: Transform::IDENTITY,
            });
            for z in [-1.0, -2.0] {
                renderer.queue_draw(DrawCommand {
                    mesh: crowd.mesh,
                    material: glass,
                    transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
                });
            }
            renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
            renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
        };

        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let first = renderer.diagnostics().clone();
        let frame = first.frame;
        // Les gains MESURABLES : 208 soumis, 200 fusionnés en UN batch,
        // 5 cullés — draws ≫ draw_calls, en chiffres.
        assert_eq!(frame.submitted, 208);
        assert_eq!(frame.resolved, 206);
        assert_eq!(frame.classic_draws, 4);
        assert_eq!(frame.instanced_draws, 1);
        assert_eq!(frame.instances, 200);
        assert_eq!(frame.culled, 5);
        assert_eq!(frame.injected, 3);
        // Les triangles : la foule 400, le masked 2, les verres 4, le
        // ciel 1 — les segments de debug comptés À PART.
        assert_eq!(frame.triangles, 407);
        assert_eq!(frame.debug_segments, 4);
        // Six pipelines se succèdent (instancié → masked → ciel →
        // transparent → debug → overlay), trois bindings distincts.
        assert_eq!(frame.pipeline_switches, 6);
        assert_eq!(frame.material_switches, 3);
        assert_eq!(frame.passes_executed, 1);
        assert_eq!(frame.passes_skipped, 0);
        // L'ombre : 201 casters (la foule + le masked — les verres
        // jamais), 5 tentatives hors volume, 2 soumissions.
        assert_eq!(
            first.shadow,
            Some(crate::diagnostics::ShadowStats {
                draws: 201,
                draw_calls: 2,
                culled: 5,
                instances: 200,
                triangles: 402,
            })
        );
        // Les coûts CPU sont MESURÉS et cohérents, le GPU du mock est
        // indisponible AVEC sa raison, les ressources sont LA photo.
        assert!(first.cpu.total_ms.is_finite() && first.cpu.total_ms >= first.cpu.backend_ms);
        assert!(matches!(
            &first.gpu,
            crate::diagnostics::GpuTiming::Unavailable { reason } if reason.contains("mock")
        ));
        assert_eq!(first.resources, renderer.resource_stats());
        assert!(first.fallbacks.fallback_textures >= 1);
        assert_eq!(first.fallbacks.degraded_permutations, 0);
        assert_eq!(first.surface.presented, 1);
        assert_eq!(first.budget.over_budget_frames, 0);
        // Le Display porte les chiffres — utilisable sans UI.
        let text = first.to_string();
        assert!(text.contains("208 submitted -> 206 resolved"));
        assert!(text.contains("gpu: unavailable"));

        // FRAME 2 : la même scène — les COMPTEURS identiques (seuls les
        // temps varient), les cumulatifs avancent.
        renderer.clear_draws();
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let second = renderer.diagnostics();
        assert_eq!(second.frame, first.frame);
        assert_eq!(second.shadow, first.shadow);
        assert_eq!(second.passes.len(), first.passes.len());
        assert_eq!(second.passes[0].draw_calls, first.passes[0].draw_calls);
        assert_eq!(second.surface.presented, 2);
    }

    #[test]
    fn render_to_target_leaves_the_diagnostics_alone() {
        let (mut renderer, _journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        let before = renderer.diagnostics().clone();
        let target = small_target(&mut renderer, "viewport");
        renderer
            .render_to_target(
                target,
                Color::BLACK,
                Mat4::IDENTITY,
                &[DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                }],
            )
            .unwrap();
        assert_eq!(renderer.diagnostics(), &before);
    }

    #[test]
    fn the_empty_plan_still_snapshots() {
        let (mut renderer, _journal) = mock_renderer();
        renderer
            .set_pass_enabled(renderer.main_pass(), false)
            .unwrap();
        renderer.render_frame().unwrap();
        let diagnostics = renderer.diagnostics();
        assert_eq!(diagnostics.frame.passes_executed, 0);
        assert_eq!(diagnostics.frame.passes_skipped, 1);
        assert_eq!(diagnostics.frame.resolved, 0);
        assert!(diagnostics.cpu.total_ms.is_finite());
        // Rien n'est parti au backend : aucun événement de surface.
        assert_eq!(diagnostics.surface.presented, 0);
    }

    #[test]
    fn debug_draws_route_by_duration_and_expire() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::marker(Vec3::ZERO, 0.5).with_duration(2.0));
        assert_eq!(
            renderer.debug_stats(),
            DebugStats {
                frame: 1,
                retained: 1
            }
        );
        // `clear_draws` vide la frame, JAMAIS les retenues.
        renderer.clear_draws();
        assert_eq!(
            renderer.debug_stats(),
            DebugStats {
                frame: 0,
                retained: 1
            }
        );
        // Le temps décompte ; un delta invalide est ignoré.
        renderer.advance_debug_time(1.0);
        renderer.advance_debug_time(f32::NAN);
        renderer.advance_debug_time(-5.0);
        assert_eq!(renderer.debug_stats().retained, 1);
        renderer.advance_debug_time(1.0);
        assert_eq!(
            renderer.debug_stats(),
            DebugStats {
                frame: 0,
                retained: 0
            }
        );
    }

    #[test]
    fn invalid_debug_draws_are_dropped_at_submit() {
        let (mut renderer, _journal) = mock_renderer();
        renderer.queue_debug(DebugDraw::line(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X));
        renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 0.0));
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X).with_duration(-1.0));
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.1).for_pass(PassHandle(42)));
        assert_eq!(renderer.debug_stats(), DebugStats::default());
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.1));
        assert_eq!(renderer.debug_stats().frame, 1);
    }

    #[test]
    fn debug_toggles_flip_the_rendering_state() {
        let (mut renderer, _journal) = mock_renderer();
        assert!(renderer.debug_enabled());
        renderer.set_debug_enabled(false);
        assert!(!renderer.debug_enabled());
        assert!(renderer.debug_category_enabled("physics"));
        renderer.set_debug_category_enabled("physics", false);
        assert!(!renderer.debug_category_enabled("physics"));
        assert!(renderer.debug_category_enabled(DEFAULT_DEBUG_CATEGORY));
        renderer.set_debug_category_enabled("physics", true);
        assert!(renderer.debug_category_enabled("physics"));
    }

    #[test]
    fn debug_is_injected_after_the_transparents() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, -1.0)),
        });
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.render_frame().unwrap();
        // Le batch debug voyage APRÈS les draws de la passe (le slot
        // réservé) — le suffixe `dbg` du journal le porte, et sa
        // permutation est lignes + blend + LessEqual sans écriture.
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains(" dbg=[v=2 p="));
        assert!(create_pipeline_lines(&journal).iter().any(|line| {
            line.starts_with("create_pipeline chaos.debug ")
                && line.contains(" blend=alpha")
                && line.contains(" depth=less_equal")
                && line.contains(" topology=lines")
        }));
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.breakdown.transparent, 1);
        assert_eq!(report.breakdown.injected, 1);
        assert_eq!(report.draws, 2);
        assert_eq!(report.draw_calls, 2);
    }

    #[test]
    fn the_overlay_batch_comes_last_with_its_own_permutation() {
        let (mut renderer, journal) = mock_renderer();
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.render_frame().unwrap();
        // Deux batches : Scene D'ABORD, l'overlay en DERNIER (il
        // dessine par-dessus) — chacun sa permutation, `.overlay` en
        // profondeur Always.
        let line = render_lines(&journal).pop().unwrap();
        let scene_at = line.find(" dbg=[v=2 p=").unwrap();
        let overlay_at = line.find(", v=6 p=").unwrap();
        assert!(scene_at < overlay_at);
        assert!(create_pipeline_lines(&journal).iter().any(|line| {
            line.starts_with("create_pipeline chaos.debug.overlay ")
                && line.contains(" depth=always")
                && line.contains(" topology=lines")
        }));
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.breakdown.injected, 2);
        assert_eq!(report.draw_calls, 2);
    }

    #[test]
    fn debug_routes_to_its_target_pass_with_its_format() {
        let (mut renderer, journal) = mock_renderer();
        let target = small_target(&mut renderer, "viewport");
        let mirror = renderer
            .add_pass(
                &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                    .with_order(-1),
            )
            .unwrap();
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::Y).for_pass(mirror));
        renderer.render_frame().unwrap();
        // Chaque passe reçoit SON debug — et la passe cible résout la
        // permutation de SON format (le descripteur, pas que le label).
        let lines = render_lines(&journal);
        assert!(lines[0].contains("pass=mirror"));
        assert!(lines[0].contains(" dbg=[v=2 p="));
        assert!(lines[1].contains(" dbg=[v=2 p="));
        assert!(create_pipeline_lines(&journal).iter().any(|line| {
            line.starts_with("create_pipeline chaos.debug.Rgba8UnormSrgb ")
                && line.contains(" target=Rgba8UnormSrgb")
        }));
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].breakdown.injected, 1);
        assert_eq!(report.passes[1].breakdown.injected, 1);
    }

    #[test]
    fn disabled_debug_leaves_the_journal_clean() {
        let (mut renderer, journal) = mock_renderer();
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 1.0).with_category("bounds"));
        // Le toggle GLOBAL coupe tout : la ligne render est EXACTEMENT
        // la ligne historique — zéro delta de journal.
        renderer.set_debug_enabled(false);
        renderer.render_frame().unwrap();
        assert_eq!(
            render_lines(&journal).pop().unwrap(),
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
        assert_eq!(renderer.frame_report().passes[0].breakdown.injected, 0);
        // La CATÉGORIE filtre au rendu : `bounds` coupée, la ligne
        // reste — un seul batch.
        renderer.set_debug_enabled(true);
        renderer.set_debug_category_enabled("bounds", false);
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains(" dbg=[v=2 p="));
        assert!(!line.contains(", v="));
        // Réveillée, la sphère revient (144 sommets + 2 de la ligne).
        renderer.set_debug_category_enabled("bounds", true);
        renderer.render_frame().unwrap();
        assert!(
            render_lines(&journal)
                .pop()
                .unwrap()
                .contains(" dbg=[v=146 p=")
        );
    }

    #[test]
    fn retained_debug_survives_clear_draws_and_expires_on_screen() {
        let (mut renderer, journal) = mock_renderer();
        renderer.queue_debug(DebugDraw::marker(Vec3::ZERO, 0.5).with_duration(5.0));
        renderer.render_frame().unwrap();
        assert!(render_lines(&journal).pop().unwrap().contains(" dbg=["));
        // `clear_draws` (la frame de simulation suivante) : la retenue
        // est TOUJOURS dessinée.
        renderer.clear_draws();
        renderer.render_frame().unwrap();
        assert!(render_lines(&journal).pop().unwrap().contains(" dbg=["));
        // Le temps l'expire : plus aucun batch, la ligne redevient
        // historique.
        renderer.advance_debug_time(6.0);
        renderer.render_frame().unwrap();
        assert!(!render_lines(&journal).pop().unwrap().contains(" dbg=["));
    }

    #[test]
    fn checkpoint_debug_v1_the_visual_language_lives_and_expires() {
        // LE checkpoint Debug Rendering V1 : toutes les formes sous les
        // deux modes de profondeur, une scène régulière à côté, une
        // retenue qui survit aux frames et expire par le temps, les
        // catégories togglées à chaud, les comptes exacts — et AUCUN
        // pipeline de plus une fois les permutations chaudes.
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "prop");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        let queue_scene = |renderer: &mut Renderer| {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            });
            renderer.queue_debug(DebugDraw::grid(Vec3::ZERO, 10.0, 1.0));
            renderer.queue_debug(DebugDraw::axes(Mat4::IDENTITY, 2.0).overlay());
            renderer.queue_debug(DebugDraw::aabb(
                Aabb::from_points([Vec3::ZERO, Vec3::ONE]).unwrap(),
            ));
            renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 1.0).with_category("bounds"));
            renderer.queue_debug(DebugDraw::frustum(projection::orthographic(
                -1.0, 1.0, -1.0, 1.0, 0.0, 10.0,
            )));
            renderer.queue_debug(DebugDraw::ray(Vec3::ZERO, Vec3::X));
            renderer.queue_debug(DebugDraw::arrow(Vec3::ZERO, Vec3::Y));
            renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.2));
            renderer.queue_debug(DebugDraw::light(
                &Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0),
                Vec3::new(0.0, 5.0, 0.0),
            ));
            renderer.queue_debug(DebugDraw::light(
                &Light::point(Vec3::X, Color::WHITE, 1.0, 2.0),
                Vec3::ZERO,
            ));
            renderer.queue_debug(DebugDraw::light(
                &Light::spot(Vec3::Y, Vec3::NEG_Y, Color::WHITE, 1.0, 3.0, 0.3, 0.5),
                Vec3::ZERO,
            ));
        };

        // FRAME 1 : 11 primitives immédiates + 1 retenue (3 s) — DEUX
        // batches (Scene puis Overlay), 12 injectées, 3 soumissions (le
        // cube + les deux batches).
        queue_scene(&mut renderer);
        renderer.queue_debug(
            DebugDraw::marker(Vec3::ZERO, 0.5)
                .with_duration(3.0)
                .with_category("markers"),
        );
        renderer.render_frame().unwrap();
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.breakdown.injected, 12);
        assert_eq!(report.draws, 13);
        assert_eq!(report.draw_calls, 3);
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("v=").count(), 2);
        let pipelines_after_first = create_pipeline_lines(&journal).len();

        // FRAME 2 : la simulation avance (clear + 1 s) — seule la
        // RETENUE survit, sur un batch Scene, sans pipeline de plus.
        renderer.clear_draws();
        renderer.advance_debug_time(1.0);
        renderer.render_frame().unwrap();
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.breakdown.injected, 1);
        assert_eq!(report.draws, 1);
        assert_eq!(report.draw_calls, 1);
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_after_first);

        // FRAME 3 : sa catégorie coupée — plus rien à l'écran, la
        // retenue continue d'expirer en coulisses.
        renderer.set_debug_category_enabled("markers", false);
        renderer.advance_debug_time(1.0);
        renderer.render_frame().unwrap();
        assert_eq!(renderer.frame_report().passes[0].breakdown.injected, 0);
        assert_eq!(renderer.debug_stats().retained, 1);

        // FRAME 4 : réveillée puis EXPIRÉE — le journal redevient
        // exactement historique.
        renderer.set_debug_category_enabled("markers", true);
        renderer.advance_debug_time(1.5);
        renderer.render_frame().unwrap();
        assert_eq!(renderer.debug_stats().retained, 0);
        assert_eq!(
            render_lines(&journal).pop().unwrap(),
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
    }

    #[test]
    fn render_to_target_draws_no_debug() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        let target = small_target(&mut renderer, "viewport");
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer
            .render_to_target(
                target,
                Color::BLACK,
                Mat4::IDENTITY,
                &[DrawCommand {
                    mesh,
                    material,
                    transform: Transform::IDENTITY,
                }],
            )
            .unwrap();
        // Le chemin immédiat ne dessine PAS de debug — la règle de la
        // passe d'ombre.
        assert!(!render_lines(&journal).pop().unwrap().contains(" dbg=["));
    }

    #[test]
    fn an_out_of_view_draw_is_culled_from_the_pass() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        // Hors du champ de la caméra large (±1000) : jamais résolu,
        // jamais soumis — le plan part VIDE, le rapport dit pourquoi.
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.ends_with("vp=[0, 0, 0] draws=[]"));
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.draws, 0);
        assert_eq!(report.draw_calls, 0);
        assert_eq!(report.culled, 1);
        assert_eq!(report.breakdown, DrawBreakdown::default());
    }

    #[test]
    fn a_straddling_draw_is_never_wrongly_rejected() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        // À cheval sur le bord droit du frustum (x = 1000, bounds
        // ±0.5) : partiellement visible → JAMAIS rejeté — le
        // conservatisme est le contrat.
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(1000.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains("m=[1000, 0, 0]"));
        assert_eq!(renderer.frame_report().passes[0].culled, 0);
    }

    #[test]
    fn a_boundless_mesh_is_never_culled() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        // Une position non finie refuse les bounds à la création : le
        // mesh n'en a pas — le défaut SÛR le dessine toujours, même
        // très loin hors champ.
        let mesh = renderer
            .create_mesh(
                "broken",
                &Geometry {
                    vertices: vec![
                        ColorVertex {
                            position: [f32::NAN, 0.0, 0.0],
                            color: [1.0, 1.0, 1.0],
                        },
                        ColorVertex {
                            position: [1.0, 0.0, 0.0],
                            color: [1.0, 1.0, 1.0],
                        },
                        ColorVertex {
                            position: [0.0, 1.0, 0.0],
                            color: [1.0, 1.0, 1.0],
                        },
                    ],
                    indices: Vec::new(),
                },
            )
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert!(line.contains("m=[5000, 0, 0]"));
        assert_eq!(renderer.frame_report().passes[0].culled, 0);
    }

    #[test]
    fn an_unculled_material_ignores_every_frustum() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_directional_shadow(&demo_shadow()).unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let ghost = renderer
            .create_material(
                &MaterialDescriptor::new("ghost", MaterialModel::Lit).without_frustum_culling(),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        // Hors caméra (±1000) ET hors volume de lumière (±10) : le
        // forcé-visible saute les DEUX tests — dessiné ET moissonné.
        renderer.queue_draw(DrawCommand {
            mesh,
            material: ghost,
            transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        assert!(
            render_lines(&journal)
                .pop()
                .unwrap()
                .contains("m=[5000, 0, 0]")
        );
        assert!(
            shadow_lines(&journal)
                .pop()
                .unwrap()
                .contains("m=[5000, 0, 0]")
        );
        assert_eq!(renderer.frame_report().passes[0].culled, 0);
        assert_eq!(renderer.frame_report().shadow.unwrap().culled, 0);
    }

    #[test]
    fn transparents_are_culled_before_the_sort() {
        let (mut renderer, journal) = mock_renderer();
        let glass = renderer
            .create_material(
                &MaterialDescriptor::new("glass", MaterialModel::Lit)
                    .with_opacity(MaterialOpacity::Transparent),
            )
            .unwrap();
        let mesh = lit_quad_mesh(&mut renderer, "pane");
        for translation in [
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(5000.0, 0.0, 0.0),
        ] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: glass,
                transform: Transform::from_translation(translation),
            });
        }
        renderer.render_frame().unwrap();
        // Le hors-champ sort AVANT le tri arrière → avant : deux
        // transparents triés, le troisième compté cullé.
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 2);
        assert!(!line.contains("5000"));
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.breakdown.transparent, 2);
        assert_eq!(report.culled, 1);
    }

    #[test]
    fn instancing_only_fuses_the_visible() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "crowd");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        // Dix objets compatibles, un sur deux hors champ (x = 0..2250,
        // la caméra large s'arrête à ±1000) : le run instancié ne
        // contient QUE les visibles — le culling se joue AVANT la
        // fusion.
        for index in 0..10u32 {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(250.0 * index as f32, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
        assert!(line.contains("inst=5"));
        let report = &renderer.frame_report().passes[0];
        assert_eq!(report.draws, 5);
        assert_eq!(report.draw_calls, 1);
        assert_eq!(report.culled, 5);
    }

    #[test]
    fn a_caster_off_screen_keeps_its_shadow_and_vice_versa() {
        // L'ANTI-POP : la moisson d'ombre teste le frustum de la
        // LUMIÈRE, jamais celui de la passe — un caster sorti de
        // l'écran projette encore, un visible hors volume ne projette
        // plus (et son ombre au sol disparaît avec le volume, pas avec
        // la caméra).
        let (mut renderer, journal) = mock_renderer();
        // Une caméra étroite : le cube NDC décalé — visible x ∈ [4, 6].
        renderer.set_view_projection(Mat4::from_translation(Vec3::new(-5.0, 0.0, 0.0)));
        renderer
            .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::ZERO,
                Vec3::new(2.0, 2.0, 2.0),
            )))
            .unwrap();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        let hidden = lit_caster(&mut renderer, "hidden");
        let lonely = lit_caster(&mut renderer, "lonely");
        renderer.queue_draw(hidden);
        renderer.queue_draw(DrawCommand {
            transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.5)),
            ..lonely
        });
        renderer.render_frame().unwrap();
        // `hidden` (origine) : hors caméra, dans le volume — ABSENT de
        // la passe, PRÉSENT dans l'ombre.
        let render = render_lines(&journal).pop().unwrap();
        assert!(render.contains("m=[5, 0, 0.5]"));
        assert!(!render.contains("m=[0, 0, 0]"));
        // `lonely` (x = 5) : visible, hors volume — l'inverse.
        let shadow = shadow_lines(&journal).pop().unwrap();
        assert!(shadow.contains("m=[0, 0, 0]"));
        assert!(!shadow.contains("m=[5, 0, 0.5]"));
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].draws, 1);
        assert_eq!(report.passes[0].culled, 1);
        assert_eq!(
            report.shadow,
            Some(ShadowReport {
                draws: 1,
                draw_calls: 1,
                culled: 1,
                resolution: 2048
            })
        );
    }

    #[test]
    fn each_view_culls_with_its_own_frustum() {
        let (mut renderer, journal) = mock_renderer();
        let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
        renderer
            .set_pass_camera(overlay, Mat4::from_translation(Vec3::new(-5.0, 0.0, 0.0)))
            .unwrap();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        let near = DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        };
        let far = DrawCommand {
            transform: Transform::from_translation(Vec3::new(500.0, 0.0, 0.0)),
            ..near
        };
        for command in [near, far] {
            renderer.queue_draw(command);
            renderer.queue_draw_to(overlay, command).unwrap();
        }
        renderer.render_frame().unwrap();
        // La caméra large de la principale voit les deux ; l'overlay
        // (cube NDC décalé, x ∈ [4, 6]) ne garde que le proche —
        // chaque passe cull avec SA vue, jamais celle d'une autre.
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].draws, 2);
        assert_eq!(report.passes[0].culled, 0);
        assert_eq!(report.passes[1].draws, 1);
        assert_eq!(report.passes[1].culled, 1);
        let lines = render_lines(&journal);
        assert!(lines[1].contains("m=[5, 0, 0]"));
        assert!(!lines[1].contains("m=[500, 0, 0]"));
    }

    #[test]
    fn checkpoint_culling_v1_a_stress_scene_pays_only_for_the_visible() {
        // LE checkpoint Culling V1 : mille et un objets dont ~900 hors
        // champ — la passe ne paie QUE les visibles (résolution,
        // instances, soumissions), l'ombre garde SES casters (celui
        // derrière la caméra projette encore), et deux frames rendent
        // EXACTEMENT les mêmes comptes sans pipeline de plus.
        let (mut renderer, journal) = mock_renderer();
        renderer
            .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::new(50.0, 0.0, 0.0),
                Vec3::new(60.0, 20.0, 60.0),
            )))
            .unwrap();
        let crowd = lit_caster(&mut renderer, "crowd");
        let queue_scene = |renderer: &mut Renderer| {
            renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
            for index in 0..1000u32 {
                let x = if index < 100 {
                    index as f32
                } else {
                    5000.0 + index as f32
                };
                renderer.queue_draw(DrawCommand {
                    transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
                    ..crowd
                });
            }
            // Le 1001e : DERRIÈRE la caméra (z > 0), dans le volume de
            // lumière (le z monde vit dans l'extent Y du volume — la
            // lumière est verticale) — son ombre reste au sol.
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(50.0, 0.0, 15.0)),
                ..crowd
            });
        };

        // FRAME 1 : 1001 objets logiques → 100 résolus, UNE soumission ;
        // l'ombre en résout 101 (les visibles + celui derrière).
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        let report = renderer.frame_report();
        assert_eq!(report.passes[0].draws, 100);
        assert_eq!(report.passes[0].draw_calls, 1);
        assert_eq!(report.passes[0].culled, 901);
        assert_eq!(
            report.shadow,
            Some(ShadowReport {
                draws: 101,
                draw_calls: 1,
                culled: 900,
                resolution: 2048
            })
        );
        let render = render_lines(&journal).pop().unwrap();
        assert_eq!(render.matches("m=[").count(), 1);
        assert!(render.contains("inst=100"));
        assert!(shadow_lines(&journal).pop().unwrap().contains("inst=101"));

        // FRAME 2 : mêmes soumissions — mêmes comptes exacts, AUCUN
        // pipeline de plus.
        let pipelines_before = create_pipeline_lines(&journal).len();
        renderer.clear_draws();
        queue_scene(&mut renderer);
        renderer.render_frame().unwrap();
        assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
        assert_eq!(renderer.frame_report().passes[0].culled, 901);
        assert_eq!(renderer.frame_report().shadow.unwrap().draws, 101);
    }

    #[test]
    fn render_to_target_culls_with_its_own_camera() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        let target = small_target(&mut renderer, "viewport");
        let inside = DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        };
        let outside = DrawCommand {
            transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
            ..inside
        };
        // Le rendu immédiat cull avec la VP QU'ON lui donne (le cube
        // NDC de l'identité) — jamais la caméra principale.
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[inside, outside])
            .unwrap();
        let line = render_lines(&journal).pop().unwrap();
        assert_eq!(line.matches("m=[").count(), 1);
        assert!(line.contains("m=[0, 0, 0]"));
        assert!(!line.contains("5000"));
    }
}
