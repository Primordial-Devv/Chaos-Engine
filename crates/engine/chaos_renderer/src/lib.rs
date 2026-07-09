//! Renderer de Chaos Engine : API graphique du moteur, backend interchangeable.
//! Architecture détaillée : `docs/renderer/overview.md`.

pub mod backend;
pub mod config;
pub mod frame;
pub mod geometry;
pub mod mesh;
mod pool;
pub mod queue;
pub mod renderer;
pub mod resources;
pub mod shaders;
pub mod target;

pub use backend::GraphicsBackend;
pub use config::RendererConfig;
pub use frame::{DrawCommand, FrameDraw, FrameOutcome, FramePlan, FrameSkipReason};
pub use geometry::Geometry;
pub use mesh::MeshHandle;
pub use queue::RenderQueue;
pub use renderer::Renderer;
pub use resources::{
    BufferDescriptor, BufferHandle, BufferKind, ColorVertex, CullMode, FrontFace,
    PipelineDescriptor, PipelineHandle, PrimitiveTopology, ShaderRef, ShaderSource,
    VertexAttribute, VertexAttributeFormat, VertexLayout, VertexStepMode, bytes_of_f32,
};
pub use shaders::ShaderLibrary;
pub use target::SurfaceTarget;
