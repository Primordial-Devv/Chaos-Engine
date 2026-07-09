use std::fmt::Display;

use chaos_core::math::Mat4;
use chaos_core::{ChaosError, Color};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::resources::{
    CullMode, FrontFace, PrimitiveTopology, VertexAttributeFormat, VertexLayout, VertexStepMode,
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
