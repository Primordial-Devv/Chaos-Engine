pub mod assets;
pub mod config;
pub mod context;
pub mod debug;
pub mod engine;
mod render_subsystem;
pub mod subsystem;

pub use chaos_core::{
    Camera, ChaosError, ChaosResult, Color, Entity, Event, InputEvent, KeyCode, Perspective, Time,
    Transform, WindowEvent, math,
};
pub use chaos_ecs::{Commands, Component, Message, Resource, Schedule, System, Systems, World};
pub use chaos_renderer::{
    ColorVertex, CullMode, DrawCommand, Geometry, MaterialDescriptor, MaterialHandle, MeshHandle,
    PipelineDescriptor, PipelineHandle, Renderer, SamplerAddressMode, SamplerDescriptor,
    SamplerFilter, SamplerHandle, ShaderSource, TextureDescriptor, TextureFormat, TextureHandle,
    TextureUsage, TexturedGeometry, TexturedVertex, VertexAttributeFormat, VertexLayout, shaders,
    srgb8_bytes_of,
};
pub use chaos_window::WindowConfig;
pub use config::EngineConfig;
pub use context::EngineContext;
pub use engine::{Engine, stages};
pub use subsystem::Subsystem;
