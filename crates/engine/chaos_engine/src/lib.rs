//! LA FAÇADE de Chaos Engine — l'unique surface que voient les
//! applications (sandbox, futur éditeur, futur serveur dédié, futur
//! runtime), verrouillée en CI : jamais un import de crate interne.
//!
//! Le contrat d'application, six points :
//!
//! 1. **Création** : `Engine::new(config)` — un cycle de vie par Engine.
//! 2. **Configuration** : `EngineConfig` (groupes `app`/`window`/`render`/
//!    `time`/`logs`/`runtime`) — des défauts sûrs, surchargés par l'app,
//!    VALIDÉS avant toute initialisation partielle.
//! 3. **Enregistrement** : `add_subsystem` (l'ordre d'init) ; les
//!    extensions (importeurs d'assets, systèmes ECS) s'enregistrent dans
//!    `Subsystem::init` via les services du contexte.
//! 4. **Démarrage** : `run()` — bloquant jusqu'à l'arrêt propre ; le même
//!    core en fenêtré (main thread) ou headless (`runtime.headless`).
//! 5. **Arrêt** : par requête (`EngineContext::request_exit`), par
//!    `runtime.frame_limit`, par fermeture système — toujours l'arrêt
//!    ordonné avec garanties de libération.
//! 6. **Résultats** : le `ChaosResult` de `run()` (la première défaillance
//!    fatale, précise) + `diagnostics()`/`metrics()` (profil CPU, santé,
//!    compteurs) lisibles après le run.
//!
//! Le modèle complet : `docs/architecture/engine-loop.md`.

pub mod assets;
pub mod config;
pub mod context;
pub mod debug;
pub mod diagnostics;
pub mod engine;
pub mod metrics;
mod render_subsystem;
pub mod scenes;
pub mod subsystem;

pub use chaos_core::{
    AssetId, Camera, ChaosError, ChaosResult, Color, ElementState, Entity, Event, FixedTime,
    GlobalTransform, InputEvent, KeyCode, Perspective, SceneId, Time, Transform, WindowEvent, math,
};
pub use chaos_ecs::{Commands, Component, Message, Resource, Schedule, System, Systems, World};
pub use chaos_renderer::{
    BuiltinTexture, CapabilityDecision, CapabilityStatus, ColorVertex, CullMode, DebugDepth,
    DebugDraw, DebugShape, DebugStats, DeviceLimits, DirectionalShadowDescriptor,
    DirectionalShadowInfo, DrawBreakdown, DrawCommand, EnvironmentDescriptor, EnvironmentInfo,
    FrameReport, Geometry, GpuTiming, KindStats, Light, LitGeometry, LitVertex, MAX_LIGHTS,
    MaterialDescriptor, MaterialHandle, MaterialInfo, MaterialModel, MaterialOpacity, MeshHandle,
    PassHandle, PassLoad, PassOutcome, PassReport, RenderDestination, RenderPassDescriptor,
    RenderTargetDescriptor, RenderTargetHandle, Renderer, RendererCapabilities,
    RendererDiagnostics, ResourceStats, SamplerAddressMode, SamplerDescriptor, SamplerFilter,
    SamplerHandle, ShaderSource, ShadowReport, ShadowVolume, TextureDescriptor, TextureFormat,
    TextureHandle, TextureKind, TextureMips, TextureUsage, TexturedGeometry, TexturedVertex,
    VertexAttributeFormat, VertexLayout, light_view_projection, rgba16f_bytes_of, shaders,
    srgb8_bytes_of,
};
pub use chaos_scene::{
    ChildOf, EntityData, FORMAT_VERSION, MeshRef, Prefab, Scene, SceneData, SceneManager,
    SceneMember, SceneState, hierarchy,
};
pub use chaos_window::WindowConfig;
pub use config::{AppConfig, EngineConfig, LogConfig, RenderConfig, RuntimeConfig, TimeConfig};
pub use context::EngineContext;
pub use diagnostics::{FrameDiagnostics, FrameProfile, Span};
pub use engine::{Engine, stages};
pub use metrics::{EngineMetrics, MetricsSnapshot, SubsystemState, SubsystemStatus};
pub use subsystem::Subsystem;
