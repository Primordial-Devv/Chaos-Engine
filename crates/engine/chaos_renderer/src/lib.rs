//! Renderer de Chaos Engine : API graphique du moteur, backend interchangeable.
//! Architecture détaillée : `docs/renderer/overview.md`.

pub mod backend;
pub mod config;
pub mod frame;
pub mod geometry;
pub mod material;
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
pub use geometry::{Geometry, TexturedGeometry};
pub use material::{MaterialDescriptor, MaterialHandle};
pub use mesh::MeshHandle;
pub use queue::RenderQueue;
pub use renderer::Renderer;
pub use resources::{
    BufferDescriptor, BufferHandle, BufferKind, ColorVertex, CullMode, FrontFace,
    MaterialBindingDescriptor, MaterialBindingHandle, PipelineDescriptor, PipelineHandle,
    PrimitiveTopology, SamplerAddressMode, SamplerDescriptor, SamplerFilter, SamplerHandle,
    ShaderRef, ShaderSource, TextureDescriptor, TextureFormat, TextureHandle, TextureUsage,
    TexturedVertex, VertexAttribute, VertexAttributeFormat, VertexLayout, VertexStepMode,
    bytes_of_f32, rgba8_bytes_of, srgb8_bytes_of,
};
pub use shaders::ShaderLibrary;
pub use target::SurfaceTarget;
