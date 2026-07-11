use std::fmt::Display;

use chaos_core::math::{Mat4, Vec3};
use chaos_core::{ChaosError, Color};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::frame::{FrameEnvironment, FrameShadowPass, InstanceTransforms};
use crate::light::{FrameLights, Light, MAX_LIGHTS};
use crate::resources::{
    CullMode, DebugVertex, DepthCompare, FrontFace, MaterialParams, PrimitiveTopology,
    SamplerAddressMode, SamplerFilter, TextureFormat, TextureUsage, VertexAttributeFormat,
    VertexLayout, VertexStepMode,
};
use crate::target::SurfaceTarget;

use super::uniforms::{FRAME_UNIFORMS_SIZE, LIGHTS_UNIFORMS_SIZE, OBJECT_UNIFORMS_SIZE};

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
    vec4_to_bytes([color.r, color.g, color.b, color.a])
}

pub(super) fn vec4_to_bytes(values: [f32; 4]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (index, value) in values.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

/// Les paramètres material packés (48 octets) — le miroir exact de
/// `MaterialUniforms` dans les WGSL : base_color @0, (metallic,
/// roughness, receive_shadows, alpha_cutoff) @16 — le vec4 de
/// paramètres est COMPLET —, émissif @32. L'alpha de l'émissif est
/// packé à 0 explicitement (les shaders l'ignorent).
pub(super) fn material_params_to_bytes(params: &MaterialParams) -> [u8; 48] {
    let mut bytes = [0u8; 48];
    bytes[..16].copy_from_slice(&color_to_bytes(params.base_color));
    bytes[16..32].copy_from_slice(&vec4_to_bytes([
        params.metallic,
        params.roughness,
        if params.receive_shadows { 1.0 } else { 0.0 },
        params.alpha_cutoff,
    ]));
    bytes[32..].copy_from_slice(&vec4_to_bytes([
        params.emissive.r,
        params.emissive.g,
        params.emissive.b,
        0.0,
    ]));
    bytes
}

/// Les uniforms de frame packés (160 octets) : la matrice vue-projection
/// @0, la position caméra @64 (vec4, w inutilisé), la vue-projection
/// INVERSE @80 (la déprojection du ciel — une matrice singulière produit
/// une inverse non finie, donc des directions NaN, visibles seulement
/// avec un environnement actif) et les paramètres d'environnement @144
/// (intensité, exposition) — le miroir de `FrameUniforms` dans les WGSL.
pub(super) fn frame_to_bytes(
    view_projection: Mat4,
    camera_position: Vec3,
    environment: &FrameEnvironment,
) -> [u8; FRAME_UNIFORMS_SIZE] {
    let mut bytes = [0u8; FRAME_UNIFORMS_SIZE];
    bytes[..64].copy_from_slice(&mat4_to_bytes(view_projection));
    bytes[64..80].copy_from_slice(&vec4_to_bytes([
        camera_position.x,
        camera_position.y,
        camera_position.z,
        0.0,
    ]));
    bytes[80..144].copy_from_slice(&mat4_to_bytes(view_projection.inverse()));
    bytes[144..].copy_from_slice(&vec4_to_bytes([
        environment.intensity,
        environment.exposure,
        0.0,
        0.0,
    ]));
    bytes
}

/// Les transforms d'instances packées pour l'instance buffer — le
/// miroir EXACT du layout d'instance (`instance_transforms_layout()`,
/// stride 128) : matrice modèle @0, matrice des normales @64, par
/// instance.
pub(super) fn instances_to_bytes(instances: &[InstanceTransforms]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(instances.len() * 128);
    for instance in instances {
        bytes.extend_from_slice(&mat4_to_bytes(instance.model));
        bytes.extend_from_slice(&mat4_to_bytes(instance.normal));
    }
    bytes
}

/// Les sommets de DEBUG packés pour le buffer de debug — le miroir
/// exact de `DebugVertex::layout()` (stride 28) : la sérialisation vit
/// chez le vertex, la frontière ne fait que l'appeler.
pub(super) fn debug_vertices_to_bytes(vertices: &[DebugVertex]) -> Vec<u8> {
    DebugVertex::bytes_of(vertices)
}

/// Les uniforms d'objet packés : matrice modèle puis matrice des
/// normales — le miroir de `ObjectUniforms` dans les WGSL intégrés.
pub(super) fn object_to_bytes(model: Mat4, normal: Mat4) -> [u8; OBJECT_UNIFORMS_SIZE] {
    let mut bytes = [0u8; OBJECT_UNIFORMS_SIZE];
    bytes[..64].copy_from_slice(&mat4_to_bytes(model));
    bytes[64..].copy_from_slice(&mat4_to_bytes(normal));
    bytes
}

/// L'éclairage de frame packé pour le buffer uniform du groupe(0)
/// binding(1) — le miroir EXACT de `LightsUniforms` dans
/// `shaders/lit.wgsl` : `ambient` (rgb + intensité) @0, `count` @16,
/// puis `MAX_LIGHTS` entrées de 64 octets (position+portée,
/// direction+genre, couleur+intensité, cône), puis la queue OMBRE —
/// la vue-projection de lumière @1056 et les paramètres @1120
/// (enabled, biais de normale, biais de profondeur, index de la
/// lumière) ; zéros quand le plan ne porte pas de passe d'ombre
/// (`enabled` 0 = facteur 1 dans les shaders).
pub(super) fn lights_to_bytes(
    lights: &FrameLights,
    shadow: Option<&FrameShadowPass>,
) -> [u8; LIGHTS_UNIFORMS_SIZE] {
    let mut bytes = [0u8; LIGHTS_UNIFORMS_SIZE];
    let mut write_vec4 = |offset: usize, values: [f32; 4]| {
        for (index, value) in values.iter().enumerate() {
            let at = offset + index * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        }
    };
    write_vec4(
        0,
        [
            lights.ambient_color.r,
            lights.ambient_color.g,
            lights.ambient_color.b,
            lights.ambient_intensity,
        ],
    );
    for (index, light) in lights.lights.iter().take(MAX_LIGHTS).enumerate() {
        let base = 32 + index * 64;
        let (position, direction, kind, color, intensity, range, cone) = match light {
            Light::Directional {
                direction,
                color,
                intensity,
                ..
            } => (
                Vec3::ZERO,
                *direction,
                0.0,
                *color,
                *intensity,
                1.0,
                [0.0, 0.0],
            ),
            Light::Point {
                position,
                color,
                intensity,
                range,
                ..
            } => (
                *position,
                Vec3::ZERO,
                1.0,
                *color,
                *intensity,
                *range,
                [0.0, 0.0],
            ),
            Light::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                inner_angle,
                outer_angle,
                ..
            } => (
                *position,
                *direction,
                2.0,
                *color,
                *intensity,
                *range,
                [inner_angle.cos(), outer_angle.cos()],
            ),
        };
        write_vec4(base, [position.x, position.y, position.z, range]);
        write_vec4(base + 16, [direction.x, direction.y, direction.z, kind]);
        write_vec4(base + 32, [color.r, color.g, color.b, intensity]);
        write_vec4(base + 48, [cone[0], cone[1], 0.0, 0.0]);
    }
    let count = u32::try_from(lights.lights.len().min(MAX_LIGHTS)).unwrap_or(0);
    bytes[16..20].copy_from_slice(&count.to_ne_bytes());
    if let Some(shadow) = shadow {
        bytes[1056..1120].copy_from_slice(&mat4_to_bytes(shadow.view_projection));
        bytes[1120..1136].copy_from_slice(&vec4_to_bytes([
            1.0,
            shadow.normal_bias,
            shadow.depth_bias,
            shadow.light_index as f32,
        ]));
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

pub(super) fn to_wgpu_depth_compare(compare: DepthCompare) -> wgpu::CompareFunction {
    match compare {
        DepthCompare::Less => wgpu::CompareFunction::Less,
        DepthCompare::LessEqual => wgpu::CompareFunction::LessEqual,
        DepthCompare::Always => wgpu::CompareFunction::Always,
    }
}

pub(super) fn to_wgpu_texture_format(format: TextureFormat) -> wgpu::TextureFormat {
    match format {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        TextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
        TextureFormat::Rg8Unorm => wgpu::TextureFormat::Rg8Unorm,
        TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
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

pub(super) fn to_wgpu_mipmap_filter_mode(filter: SamplerFilter) -> wgpu::MipmapFilterMode {
    match filter {
        SamplerFilter::Nearest => wgpu::MipmapFilterMode::Nearest,
        SamplerFilter::Linear => wgpu::MipmapFilterMode::Linear,
    }
}

pub(super) fn to_wgpu_address_mode(mode: SamplerAddressMode) -> wgpu::AddressMode {
    match mode {
        SamplerAddressMode::Repeat => wgpu::AddressMode::Repeat,
        SamplerAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        SamplerAddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
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

    #[test]
    fn instance_bytes_mirror_the_instance_layout() {
        // 128 octets par instance : modèle @0, normale @64 — le miroir
        // du layout d'instance (stride verrouillé côté resources).
        let instances = [
            InstanceTransforms {
                model: Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                normal: Mat4::IDENTITY,
            },
            InstanceTransforms {
                model: Mat4::IDENTITY,
                normal: Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0)),
            },
        ];
        let bytes = instances_to_bytes(&instances);
        assert_eq!(bytes.len(), 256);
        assert_eq!(bytes[..64], mat4_to_bytes(instances[0].model));
        assert_eq!(bytes[64..128], mat4_to_bytes(instances[0].normal));
        assert_eq!(bytes[128..192], mat4_to_bytes(instances[1].model));
        assert_eq!(bytes[192..256], mat4_to_bytes(instances[1].normal));
    }

    #[test]
    fn lights_bytes_carry_the_shadow_tail() {
        let lights = FrameLights::default();
        // Sans passe d'ombre : la queue est ENTIÈREMENT à zéro —
        // `enabled` 0 = facteur 1 dans les shaders.
        let bytes = lights_to_bytes(&lights, None);
        assert_eq!(bytes.len(), LIGHTS_UNIFORMS_SIZE);
        assert!(bytes[1056..].iter().all(|byte| *byte == 0));
        // Avec une passe d'ombre : la matrice @1056, les paramètres
        // @1120 (enabled, biais de normale, biais de profondeur, index).
        let shadow = crate::frame::FrameShadowPass {
            view_projection: Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0)),
            resolution: 2048,
            depth_bias: 0.25,
            normal_bias: 0.5,
            light_index: 3,
            draws: Vec::new(),
            instances: Vec::new(),
        };
        let bytes = lights_to_bytes(&lights, Some(&shadow));
        assert_eq!(bytes[1056..1120], mat4_to_bytes(shadow.view_projection));
        assert_eq!(bytes[1120..1136], vec4_to_bytes([1.0, 0.5, 0.25, 3.0]));
    }
}
