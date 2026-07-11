//! LE RENDERING CORE de Chaos Engine : l'API graphique du moteur, à
//! backend interchangeable — wgpu n'est que l'implémentation actuelle,
//! confinée dans `backend/wgpu` (verrouillé en CI, sources et manifestes).
//!
//! **La carte des concepts** : un DESCRIPTEUR backend-agnostic décrit la
//! ressource → `Renderer::create_*` la valide PUIS la crée chez le
//! backend → un HANDLE opaque et générationnel l'identifie (un handle
//! périmé est détecté, jamais résolu vers une autre ressource) → les
//! `DrawCommand` de la frame s'accumulent dans la file de LEUR passe
//! déclarée (`RenderPassDescriptor` — la passe principale par défaut) →
//! le plan de frame, la suite ORDONNÉE des passes résolues, est exécuté
//! par le backend, qui produit un `FrameOutcome`.
//!
//! **Deux audiences, une frontière** :
//! - **le CONSOMMATEUR** (le moteur, demain l'éditeur) : `Renderer`,
//!   les descripteurs, les handles, `DrawCommand`, les géométries et la
//!   `ShaderLibrary` — jamais un détail d'implémentation ;
//! - **l'IMPLÉMENTEUR DE BACKEND** (un futur Vulkan/DX12/Metal natif) :
//!   `GraphicsBackend`, `FramePlan`/`FramePass`/`FrameDraw`,
//!   `SurfaceTarget` — le contrat d'exécution, en vocabulaire Chaos
//!   exclusivement.
//!
//! **Les conventions** : la validation sémantique se joue AVANT le
//! backend (jamais de données invalides côté GPU) ; chaque ressource
//! porte un label de diagnostic ; les erreurs parlent le vocabulaire
//! Chaos (`ChaosError::Graphics`) ; les chemins chauds ne loguent pas.
//!
//! **La durée de vie** : le renderer CONNAÎT ses ressources — identité,
//! état, dépendances, coût (`resource_stats`). Le partage est compté
//! (détruire une texture encore utilisée par un material est REFUSÉ en
//! nommant les dépendants), les fallbacks builtin sont protégés, et les
//! libérations backend sont DIFFÉRÉES au point sûr de fin de frame (le
//! contrat des futurs backends natifs). Limite V1 assumée : les pipelines
//! sont PERMANENTS (pas de destruction, handle non générationnel).
#![deny(missing_docs)]

/// Le contrat `GraphicsBackend` et ses implémentations (wgpu, confinée).
pub mod backend;
/// Les capacités : ce que le GPU offre, ce que le renderer en a décidé.
pub mod capabilities;
/// Les paramètres d'attachement du renderer.
pub mod config;
/// Le debug rendering : le langage visuel des données spatiales.
pub mod debug;
/// Les diagnostics : le snapshot de ce que la frame rend, élimine et coûte.
pub mod diagnostics;
/// L'environnement de scène : cubemap, intensité, ciel.
pub mod environment;
/// Les ordres de dessin, le plan de frame et les issues de frame.
pub mod frame;
/// Les géométries CPU du moteur (triangle, quad, cube — colorés, texturés).
pub mod geometry;
/// La durée de vie des ressources : registre, retraite différée, stats.
pub mod lifetime;
/// Les lumières : les données d'éclairage soumises par frame.
pub mod light;
/// Les materials : LE concept de surface (pipeline + couleur + texture).
pub mod material;
/// Les meshes : la géométrie matérialisée en buffers GPU.
pub mod mesh;
/// Les passes déclarées : l'orchestration de la frame en plan explicite.
pub mod pass;
mod pool;
/// La RenderQueue : de l'ordre de scène à l'ordre de rendu.
pub mod queue;
/// La façade `Renderer` — l'API du consommateur.
pub mod renderer;
/// Les ressources GPU : descripteurs, handles, enums du vocabulaire Chaos.
pub mod resources;
/// La bibliothèque de shaders et les conventions d'entrées WGSL.
pub mod shaders;
/// Les ombres : le volume de lumière, la configuration, la vue lumière.
pub mod shadow;
#[cfg(test)]
mod suite;
/// La surface de présentation (l'intégration fenêtre).
pub mod target;
#[cfg(test)]
mod testing;
/// La visibilité : le frustum par vue et son test de bounds.
pub mod visibility;

pub use capabilities::{CapabilityDecision, CapabilityStatus, DeviceLimits, RendererCapabilities};
pub use config::RendererConfig;
pub use debug::{DEFAULT_DEBUG_CATEGORY, DebugDepth, DebugDraw, DebugShape, DebugStats};
pub use diagnostics::{
    BudgetStats, CpuCost, FallbackStats, FrameTotals, GpuTiming, PassStats, RendererDiagnostics,
    ShadowStats, SurfaceStats,
};
pub use environment::{EnvironmentDescriptor, EnvironmentInfo};
pub use frame::{DrawCommand, FrameOutcome, FrameSkipReason, RenderDestination};
pub use geometry::{Geometry, LitGeometry, TexturedGeometry};
pub use lifetime::{KindStats, ResourceStats};
pub use light::{Light, MAX_LIGHTS};
pub use material::{
    MaterialDescriptor, MaterialHandle, MaterialInfo, MaterialModel, MaterialOpacity,
};
pub use mesh::MeshHandle;
pub use pass::{
    DrawBreakdown, FrameReport, PassHandle, PassLoad, PassOutcome, PassReport,
    RenderPassDescriptor, ShadowReport,
};
pub use queue::RenderQueue;
pub use renderer::Renderer;
pub use resources::{
    BufferDescriptor, BufferHandle, BufferKind, BuiltinTexture, ColorVertex, CullMode, DebugVertex,
    DepthCompare, FrontFace, LitVertex, MaterialBindingDescriptor, MaterialBindingHandle,
    MaterialParams, PipelineDescriptor, PipelineHandle, PrimitiveTopology, RenderTargetDescriptor,
    RenderTargetHandle, SamplerAddressMode, SamplerDescriptor, SamplerFilter, SamplerHandle,
    ShaderRef, ShaderSource, TextureDescriptor, TextureFormat, TextureHandle, TextureKind,
    TextureMips, TextureUsage, TexturedVertex, VertexAttribute, VertexAttributeFormat,
    VertexLayout, VertexStepMode, bytes_of_f32, instance_transforms_layout, max_mip_levels,
    mip_dimensions, rgba8_bytes_of, rgba16f_bytes_of, srgb8_bytes_of,
};
pub use shaders::ShaderLibrary;
pub use shadow::{
    DirectionalShadowDescriptor, DirectionalShadowInfo, MAX_SHADOW_RESOLUTION,
    MIN_SHADOW_RESOLUTION, ShadowVolume, light_view_projection,
};
pub use visibility::Frustum;

pub use backend::GraphicsBackend;
pub use frame::{
    FrameDebugBatch, FrameDraw, FrameEnvironment, FramePass, FramePlan, InstanceRange,
    InstanceTransforms,
};
pub use light::FrameLights;
pub use shadow::ShadowConfig;
pub use target::SurfaceTarget;
