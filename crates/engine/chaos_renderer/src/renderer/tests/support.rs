//! Les helpers partagés des tests du renderer : descripteurs, géométries,
//! materials et ressources miniatures construits en une ligne.

use super::*;

pub(super) fn inline_descriptor(label: &str) -> PipelineDescriptor {
    PipelineDescriptor::new(label, ShaderSource::Wgsl(String::from("inline-code")))
}

pub(super) fn triangle() -> Geometry {
    Geometry::triangle(
        [0.0, 0.0, 0.0],
        1.0,
        [Color::WHITE, Color::WHITE, Color::WHITE],
    )
}

pub(super) fn quad() -> Geometry {
    Geometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, Color::WHITE)
}

pub(super) fn cube() -> Geometry {
    Geometry::cube([0.0, 0.0, 0.0], 1.0, [Color::WHITE; 6])
}

pub(super) fn plain_material(renderer: &mut Renderer, label: &str) -> MaterialHandle {
    renderer
        .create_material(&MaterialDescriptor::new(label, MaterialModel::VertexColor))
        .unwrap()
}

pub(super) fn small_texture(renderer: &mut Renderer, label: &str) -> TextureHandle {
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

pub(super) fn textured_material(
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

pub(super) fn small_target(renderer: &mut Renderer, label: &str) -> RenderTargetHandle {
    renderer
        .create_render_target(&RenderTargetDescriptor::new(
            label,
            4,
            4,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap()
}

pub(super) fn texture_and_sampler(renderer: &mut Renderer) -> (TextureHandle, SamplerHandle) {
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

pub(super) fn surface_pass(label: &str) -> RenderPassDescriptor {
    RenderPassDescriptor::new(label, RenderDestination::Surface)
}

pub(super) fn lights_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("lights "))
        .collect()
}

pub(super) fn lit_quad_mesh(renderer: &mut Renderer, label: &str) -> MeshHandle {
    let quad = LitGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    renderer.create_lit_mesh(label, &quad).unwrap()
}

pub(super) fn binding_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("create_material_binding"))
        .collect()
}

pub(super) fn env_cubemap(renderer: &mut Renderer, label: &str) -> TextureHandle {
    renderer
        .create_texture(&TextureDescriptor::cube(
            label,
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 24],
        ))
        .unwrap()
}

pub(super) fn environment_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("set_environment"))
        .collect()
}

// Le tuple d'un draw ciel dans la ligne render du mock : pas de
// buffers, 3 sommets générés, pas de binding, matrice identité.
pub(super) const SKY_DRAW: &str = ", None, None, 3, b=None, m=[0, 0, 0])";

pub(super) fn demo_shadow() -> DirectionalShadowDescriptor {
    DirectionalShadowDescriptor::new(ShadowVolume::new(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0)))
}

pub(super) fn lit_caster(renderer: &mut Renderer, label: &str) -> DrawCommand {
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
