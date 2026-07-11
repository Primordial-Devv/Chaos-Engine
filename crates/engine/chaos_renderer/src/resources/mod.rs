//! Les ressources GPU du vocabulaire Chaos : descripteurs
//! backend-agnostic, handles opaques générationnels, enums maison.

/// Le binding GPU d'un material (groupe 2), côté contrat backend.
pub mod binding;
/// Les buffers GPU (vertex, index) et les helpers d'octets.
pub mod buffer;
/// Les pipelines graphiques et leurs réglages.
pub mod pipeline;
/// Les cibles de rendu hors écran (couleur + profondeur propre).
pub mod render_target;
/// Les samplers : COMMENT une texture est lue.
pub mod sampler;
/// Les sources et références de shaders.
pub mod shader;
/// Les textures (2D, cubemaps), formats, mips et builtins.
pub mod texture;
/// Les layouts déclaratifs de vertex et les vertex standards.
pub mod vertex;

pub use binding::{MaterialBindingDescriptor, MaterialBindingHandle, MaterialParams};
pub use buffer::{BufferDescriptor, BufferHandle, BufferKind, bytes_of_f32, bytes_of_u16};
pub use pipeline::{
    CullMode, DepthCompare, FrontFace, PipelineDescriptor, PipelineHandle, PrimitiveTopology,
};
pub use render_target::{RenderTargetDescriptor, RenderTargetHandle};
pub use sampler::{SamplerAddressMode, SamplerDescriptor, SamplerFilter, SamplerHandle};
pub use shader::{ShaderRef, ShaderSource};
pub use texture::{
    BuiltinTexture, TextureDescriptor, TextureFormat, TextureHandle, TextureKind, TextureMips,
    TextureUsage, max_mip_levels, mip_dimensions, rgba8_bytes_of, rgba16f_bytes_of, srgb8_bytes_of,
};
pub use vertex::{
    ColorVertex, DebugVertex, LitVertex, TexturedVertex, VertexAttribute, VertexAttributeFormat,
    VertexLayout, VertexStepMode, instance_transforms_layout,
};
