use chaos_core::ChaosResult;
use log::debug;

use crate::backend::GraphicsBackend;
use crate::config::RendererConfig;
use crate::frame::{FrameOutcome, FramePlan, FrameSkipReason};
use crate::resources::{
    BufferDescriptor, BufferHandle, PipelineDescriptor, PipelineHandle, ShaderSource,
};
use crate::target::SurfaceTarget;

mod buffer;
mod convert;
mod frame;
mod pipeline;
mod setup;

use crate::pool::ResourcePool;
use frame::Acquisition;
use setup::GpuContext;

pub(super) struct WgpuBackend {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    description: String,
    suspended: bool,
    pipelines: Vec<wgpu::RenderPipeline>,
    buffers: ResourcePool<wgpu::Buffer>,
}

impl WgpuBackend {
    pub(super) fn new(
        target: Box<dyn SurfaceTarget>,
        renderer_config: RendererConfig,
    ) -> ChaosResult<Self> {
        let GpuContext {
            surface,
            device,
            queue,
            config,
            description,
        } = setup::initialize(target, renderer_config)?;
        Ok(Self {
            surface,
            device,
            queue,
            config,
            description,
            suspended: false,
            pipelines: Vec::new(),
            buffers: ResourcePool::new(),
        })
    }
}

impl GraphicsBackend for WgpuBackend {
    fn description(&self) -> String {
        self.description.clone()
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            debug!("window has zero area, rendering suspended");
            self.suspended = true;
            return;
        }
        self.suspended = false;
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
        shader: &ShaderSource,
    ) -> ChaosResult<PipelineHandle> {
        self.build_pipeline(descriptor, shader)
    }

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle> {
        self.build_buffer(descriptor)
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        self.release_buffer(handle)
    }

    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
        if self.suspended {
            return Ok(FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
        }
        let frame = match self.acquire_frame()? {
            Acquisition::Ready(frame) => frame,
            Acquisition::Skip(reason) => return Ok(FrameOutcome::Skipped(reason)),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let commands = self.encode_frame(&view, plan);
        self.submit_and_present(frame, commands);
        Ok(FrameOutcome::Rendered)
    }
}
