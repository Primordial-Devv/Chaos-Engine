pub(super) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub(super) fn create_depth_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("chaos.depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Une profondeur ÉCHANTILLONNABLE (les shadow maps) : attachement ET
/// texture bindable, carrée — l'organe des passes d'ombre.
pub(super) fn create_sampleable_depth_view(
    device: &wgpu::Device,
    size: u32,
    label: &str,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.max(1),
            height: size.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Efface une profondeur à 1.0 (le plus lointain) par une passe dédiée.
/// `write_texture` est INTERDIT sur `Depth32Float` et wgpu zéro-initialise
/// à 0.0 — lu sous une comparaison LessEqual, ce serait « tout ombré ».
pub(super) fn clear_depth_to_one(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    label: &str,
) {
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    queue.submit(std::iter::once(encoder.finish()));
}
