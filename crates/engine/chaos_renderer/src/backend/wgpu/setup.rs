use chaos_core::{ChaosError, ChaosResult};
use log::{debug, info};

use crate::config::RendererConfig;
use crate::target::SurfaceTarget;

use super::convert::{TargetHandles, graphics_error};

pub(super) struct GpuContext {
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) config: wgpu::SurfaceConfiguration,
    pub(super) description: String,
}

pub(super) fn initialize(
    target: Box<dyn SurfaceTarget>,
    renderer_config: RendererConfig,
) -> ChaosResult<GpuContext> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

    let surface = instance
        .create_surface(TargetHandles(target))
        .map_err(graphics_error)?;

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(graphics_error)?;

    let adapter_info = adapter.get_info();
    let description = format!("wgpu ({} / {:?})", adapter_info.name, adapter_info.backend);
    info!("graphics adapter selected: {description}");

    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(graphics_error)?;

    let capabilities = surface.get_capabilities(&adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| capabilities.formats.first().copied())
        .ok_or_else(|| ChaosError::Graphics(String::from("no compatible surface format")))?;

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: renderer_config.width.max(1),
        height: renderer_config.height.max(1),
        present_mode: if renderer_config.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        },
        alpha_mode: capabilities
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: Vec::new(),
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);
    debug!(
        "surface configured: {}x{} ({format:?})",
        config.width, config.height
    );

    Ok(GpuContext {
        surface,
        device,
        queue,
        config,
        description,
    })
}
