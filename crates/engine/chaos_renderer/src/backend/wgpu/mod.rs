use chaos_core::ChaosResult;
use log::debug;

use crate::backend::GraphicsBackend;
use crate::config::RendererConfig;
use crate::frame::{FrameOutcome, FramePlan, FrameSkipReason};
use crate::resources::{
    BufferDescriptor, BufferHandle, MaterialBindingDescriptor, MaterialBindingHandle,
    PipelineDescriptor, PipelineHandle, SamplerDescriptor, SamplerHandle, ShaderSource,
    TextureDescriptor, TextureHandle,
};
use crate::target::SurfaceTarget;

mod binding;
mod buffer;
mod convert;
mod depth;
mod frame;
mod pipeline;
mod sampler;
mod setup;
mod texture;
mod uniforms;

use crate::pool::ResourcePool;
use binding::MaterialBindings;
use convert::mat4_to_bytes;
use frame::Acquisition;
use pipeline::PipelineEntry;
use setup::GpuContext;
use uniforms::Uniforms;

pub(super) struct WgpuBackend {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    description: String,
    suspended: bool,
    pipelines: Vec<PipelineEntry>,
    buffers: ResourcePool<wgpu::Buffer>,
    textures: ResourcePool<wgpu::Texture>,
    samplers: ResourcePool<wgpu::Sampler>,
    material_bindings: MaterialBindings,
    uniforms: Uniforms,
    depth_view: wgpu::TextureView,
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
        let uniforms = Uniforms::new(&device);
        let material_bindings = MaterialBindings::new(&device);
        let depth_view = depth::create_depth_view(&device, config.width, config.height);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            description,
            suspended: false,
            pipelines: Vec::new(),
            buffers: ResourcePool::new(),
            textures: ResourcePool::new(),
            samplers: ResourcePool::new(),
            material_bindings,
            uniforms,
            depth_view,
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
        self.depth_view = depth::create_depth_view(&self.device, width, height);
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

    fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle> {
        self.build_texture(descriptor)
    }

    fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        self.release_texture(handle)
    }

    fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle> {
        self.build_sampler(descriptor)
    }

    fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        self.release_sampler(handle)
    }

    fn create_material_binding(
        &mut self,
        descriptor: &MaterialBindingDescriptor,
    ) -> ChaosResult<MaterialBindingHandle> {
        self.build_material_binding(descriptor)
    }

    fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()> {
        self.release_material_binding(handle)
    }

    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
        if self.suspended {
            return Ok(FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
        }
        self.uniforms
            .write_frame(&self.queue, &mat4_to_bytes(plan.view_projection));
        self.uniforms
            .ensure_object_slots(&self.device, plan.draws.len());
        for (index, draw) in plan.draws.iter().enumerate() {
            self.uniforms
                .write_object(&self.queue, index, &mat4_to_bytes(draw.model));
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
