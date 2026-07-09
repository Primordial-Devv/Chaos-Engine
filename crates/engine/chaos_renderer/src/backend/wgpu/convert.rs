use std::fmt::Display;

use chaos_core::math::Mat4;
use chaos_core::{ChaosError, Color};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::resources::{
    CullMode, FrontFace, PrimitiveTopology, SamplerAddressMode, SamplerFilter, TextureFormat,
    TextureUsage, VertexAttributeFormat, VertexLayout, VertexStepMode,
};
use crate::target::SurfaceTarget;

pub(super) fn to_wgpu_vertex_format(format: VertexAttributeFormat) -> wgpu::VertexFormat {
    match format {
        VertexAttributeFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
        VertexAttributeFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
        VertexAttributeFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
    }
}

pub(super) fn to_wgpu_step_mode(step_mode: VertexStepMode) -> wgpu::VertexStepMode {
    match step_mode {
        VertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
        VertexStepMode::Instance => wgpu::VertexStepMode::Instance,
    }
}

pub(super) fn to_wgpu_vertex_attributes(layout: &VertexLayout) -> Vec<wgpu::VertexAttribute> {
    layout
        .attributes
        .iter()
        .map(|attribute| wgpu::VertexAttribute {
            format: to_wgpu_vertex_format(attribute.format),
            offset: u64::from(attribute.offset),
            shader_location: attribute.location,
        })
        .collect()
}

pub(super) fn graphics_error(error: impl Display) -> ChaosError {
    ChaosError::Graphics(error.to_string())
}

pub(super) fn mat4_to_bytes(matrix: Mat4) -> [u8; 64] {
    let mut bytes = [0u8; 64];
    for (index, value) in matrix.to_cols_array().iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub(super) fn color_to_bytes(color: Color) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (index, value) in [color.r, color.g, color.b, color.a].iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub(super) fn to_wgpu_color(color: Color) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color.r),
        g: f64::from(color.g),
        b: f64::from(color.b),
        a: f64::from(color.a),
    }
}

pub(super) fn to_wgpu_topology(topology: PrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        PrimitiveTopology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
        PrimitiveTopology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
        PrimitiveTopology::LineList => wgpu::PrimitiveTopology::LineList,
        PrimitiveTopology::PointList => wgpu::PrimitiveTopology::PointList,
    }
}

pub(super) fn to_wgpu_cull_mode(mode: CullMode) -> Option<wgpu::Face> {
    match mode {
        CullMode::None => None,
        CullMode::Front => Some(wgpu::Face::Front),
        CullMode::Back => Some(wgpu::Face::Back),
    }
}

pub(super) fn to_wgpu_front_face(face: FrontFace) -> wgpu::FrontFace {
    match face {
        FrontFace::Ccw => wgpu::FrontFace::Ccw,
        FrontFace::Cw => wgpu::FrontFace::Cw,
    }
}

pub(super) fn to_wgpu_texture_format(format: TextureFormat) -> wgpu::TextureFormat {
    match format {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        TextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
        TextureFormat::Rg8Unorm => wgpu::TextureFormat::Rg8Unorm,
    }
}

pub(super) fn to_wgpu_texture_usages(usage: TextureUsage) -> wgpu::TextureUsages {
    match usage {
        TextureUsage::Sampled => {
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
        }
        TextureUsage::RenderTarget => {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        }
    }
}

/// Octets d'une rangée de texels ; saturé à u32::MAX en cas de débordement —
/// wgpu rejette alors la copie sous error scope, jamais un panic.
pub(super) fn texel_row_bytes(width: u32, bytes_per_pixel: u32) -> u32 {
    u32::try_from(u64::from(width) * u64::from(bytes_per_pixel)).unwrap_or(u32::MAX)
}

pub(super) fn to_wgpu_filter_mode(filter: SamplerFilter) -> wgpu::FilterMode {
    match filter {
        SamplerFilter::Nearest => wgpu::FilterMode::Nearest,
        SamplerFilter::Linear => wgpu::FilterMode::Linear,
    }
}

pub(super) fn to_wgpu_address_mode(mode: SamplerAddressMode) -> wgpu::AddressMode {
    match mode {
        SamplerAddressMode::Repeat => wgpu::AddressMode::Repeat,
        SamplerAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
    }
}

pub(super) struct TargetHandles(pub(super) Box<dyn SurfaceTarget>);

impl HasWindowHandle for TargetHandles {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for TargetHandles {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_row_bytes_multiplies_width_by_texel_size() {
        assert_eq!(texel_row_bytes(4, 4), 16);
        assert_eq!(texel_row_bytes(3, 2), 6);
        assert_eq!(texel_row_bytes(1, 1), 1);
    }

    #[test]
    fn texel_row_bytes_saturates_instead_of_panicking() {
        assert_eq!(texel_row_bytes(u32::MAX, 4), u32::MAX);
        assert_eq!(texel_row_bytes(u32::MAX, 1), u32::MAX);
    }
}
