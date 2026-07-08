use chaos_core::{ChaosError, ChaosResult, Color};
use log::{debug, info, warn};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::backend::GraphicsBackend;
use crate::renderer::RendererConfig;
use crate::target::SurfaceTarget;

pub(crate) struct WgpuBackend {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    description: String,
}

impl WgpuBackend {
    pub(crate) fn new(
        target: Box<dyn SurfaceTarget>,
        renderer_config: RendererConfig,
    ) -> ChaosResult<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let surface = instance
            .create_surface(TargetHandles(target))
            .map_err(|e| ChaosError::Graphics(e.to_string()))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| ChaosError::Graphics(e.to_string()))?;

        let adapter_info = adapter.get_info();
        let description = format!("wgpu ({} / {:?})", adapter_info.name, adapter_info.backend);
        info!("graphics adapter selected: {description}");

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| ChaosError::Graphics(e.to_string()))?;

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

        Ok(Self {
            surface,
            device,
            queue,
            config,
            description,
        })
    }
}

impl GraphicsBackend for WgpuBackend {
    fn description(&self) -> String {
        self.description.clone()
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            debug!("resize to zero ignored");
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render_frame(&mut self, clear_color: Color) -> ChaosResult<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                debug!("suboptimal surface texture, presenting anyway");
                frame
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                warn!("surface lost or outdated, reconfiguring");
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                debug!("surface unavailable this frame, skipping");
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ChaosError::Graphics(String::from(
                    "validation error while acquiring the surface texture",
                )));
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chaos.frame"),
            });
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chaos.clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(to_wgpu_color(clear_color)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}

struct TargetHandles(Box<dyn SurfaceTarget>);

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

fn to_wgpu_color(color: Color) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color.r),
        g: f64::from(color.g),
        b: f64::from(color.b),
        a: f64::from(color.a),
    }
}
