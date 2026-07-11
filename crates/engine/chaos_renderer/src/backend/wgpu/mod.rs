use chaos_core::ChaosResult;
use chaos_core::math::Mat4;
use log::debug;

use crate::backend::GraphicsBackend;
use crate::capabilities::RendererCapabilities;
use crate::config::RendererConfig;
use crate::diagnostics::GpuTiming;
use crate::frame::{
    FrameEnvironment, FrameOutcome, FramePass, FramePlan, FrameShadowPass, FrameSkipReason,
    RenderDestination,
};
use crate::resources::{
    BufferDescriptor, BufferHandle, MaterialBindingDescriptor, MaterialBindingHandle,
    PipelineDescriptor, PipelineHandle, RenderTargetDescriptor, RenderTargetHandle,
    SamplerDescriptor, SamplerHandle, ShaderSource, TextureDescriptor, TextureHandle,
};
use crate::shadow::ShadowConfig;
use crate::target::SurfaceTarget;

mod binding;
mod buffer;
mod convert;
mod debug;
mod depth;
mod frame;
mod instances;
mod pipeline;
mod render_target;
mod sampler;
mod setup;
mod texture;
mod timing;
mod uniforms;

use crate::pool::ResourcePool;
use binding::MaterialBindings;
use debug::DebugBuffer;
use frame::Acquisition;
use instances::InstanceBuffer;
use pipeline::PipelineEntry;
use render_target::RenderTargetRecord;
use setup::GpuContext;
use timing::GpuTimer;
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
    instances: InstanceBuffer,
    debug_vertices: DebugBuffer,
    timing: Option<GpuTimer>,
    capabilities: RendererCapabilities,
    depth_view: wgpu::TextureView,
    render_targets: ResourcePool<RenderTargetRecord>,
    shadow_view: Option<wgpu::TextureView>,
    shadow_resolution: Option<u32>,
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
            timestamp_period,
            capabilities,
        } = setup::initialize(target, renderer_config)?;
        let timing = timestamp_period.map(|period| GpuTimer::new(&device, period));
        let uniforms = Uniforms::new(&device, &queue);
        let material_bindings = MaterialBindings::new(&device);
        let instances = InstanceBuffer::new(&device);
        let debug_vertices = DebugBuffer::new(&device);
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
            instances,
            debug_vertices,
            timing,
            capabilities,
            depth_view,
            render_targets: ResourcePool::new(),
            shadow_view: None,
            shadow_resolution: None,
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

    fn update_texture(&mut self, handle: TextureHandle, pixels: &[u8]) -> ChaosResult<()> {
        self.write_texture_pixels(handle, pixels)
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

    fn update_material_binding(
        &mut self,
        handle: MaterialBindingHandle,
        params: &crate::resources::MaterialParams,
    ) -> ChaosResult<()> {
        self.write_material_uniforms(handle, params)
    }

    fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()> {
        self.release_material_binding(handle)
    }

    fn set_environment(&mut self, cubemap: Option<TextureHandle>) -> ChaosResult<()> {
        let view = match cubemap {
            Some(handle) => {
                let pool_handle = crate::pool::PoolHandle {
                    index: handle.index,
                    generation: handle.generation,
                };
                let Some(texture) = self.textures.get(pool_handle) else {
                    return Err(chaos_core::ChaosError::Graphics(String::from(
                        "texture handle is stale or already destroyed",
                    )));
                };
                Some(texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("chaos.environment"),
                    dimension: Some(wgpu::TextureViewDimension::Cube),
                    ..Default::default()
                }))
            }
            None => None,
        };
        self.uniforms.rebind_environment(&self.device, view);
        Ok(())
    }

    fn set_shadow(&mut self, config: Option<ShadowConfig>) -> ChaosResult<()> {
        match config {
            Some(config) => {
                if self.shadow_resolution == Some(config.resolution) {
                    return Ok(());
                }
                let view = depth::create_sampleable_depth_view(
                    &self.device,
                    config.resolution,
                    "chaos.shadow_map",
                );
                depth::clear_depth_to_one(&self.device, &self.queue, &view, "chaos.shadow_map");
                self.uniforms
                    .rebind_shadow(&self.device, Some(view.clone()));
                self.shadow_view = Some(view);
                self.shadow_resolution = Some(config.resolution);
                debug!("shadow map created ({0}x{0})", config.resolution);
            }
            None => {
                if self.shadow_resolution.is_none() {
                    return Ok(());
                }
                self.uniforms.rebind_shadow(&self.device, None);
                self.shadow_view = None;
                self.shadow_resolution = None;
                debug!("shadow map released, fallback rebound");
            }
        }
        Ok(())
    }

    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
        // L'éclairage est constant sur le plan : UNE écriture avant la
        // boucle — stagée avant le premier submit, elle s'applique à
        // toutes les passes (le complément du contrat « écritures par
        // passe » des uniforms partagés). La queue OMBRE (vue de
        // lumière, biais) voyage dans le même buffer.
        self.uniforms.write_lights(
            &self.queue,
            &convert::lights_to_bytes(&plan.lights, plan.shadow.as_ref()),
        );
        // L'acquisition se joue AVANT la passe d'ombre : le chronométrage
        // GPU doit savoir quelles passes s'exécuteront pour poser ses
        // bornes (première/dernière) — l'ordre des SOUMISSIONS reste
        // inchangé, l'ombre part toujours en premier.
        let wants_surface = plan
            .passes
            .iter()
            .any(|pass| pass.destination == RenderDestination::Surface);
        let mut surface_frame = None;
        let mut surface_view = None;
        let mut surface_skip = None;
        if wants_surface {
            if self.suspended {
                surface_skip = Some(FrameSkipReason::ZeroArea);
            } else {
                match self.acquire_frame()? {
                    Acquisition::Ready(frame) => {
                        surface_view = Some(
                            frame
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        );
                        surface_frame = Some(frame);
                    }
                    Acquisition::Skip(reason) => surface_skip = Some(reason),
                }
            }
        }
        // Les bornes du span GPU : le DÉBUT sur la première passe
        // exécutée (l'ombre si présente), la FIN sur la dernière — le
        // span couvre la frame GPU entière, inter-submits compris.
        let executes: Vec<bool> = plan
            .passes
            .iter()
            .map(|pass| match pass.destination {
                RenderDestination::Surface => surface_view.is_some(),
                RenderDestination::Target(_) => true,
            })
            .collect();
        let first_color = executes.iter().position(|&executes| executes);
        let last_color = executes.iter().rposition(|&executes| executes);
        let timed = plan.shadow.is_some() || first_color.is_some();
        // La passe d'ombre s'exécute AVANT toutes les passes : sa
        // soumission suit le contrat inter-passes (uniforms écrits puis
        // UN submit) — les passes suivantes échantillonnent une map
        // complète, la timeline de queue garantit l'ordre.
        if let Some(shadow) = &plan.shadow {
            self.render_shadow_pass(shadow, true, last_color.is_none());
        }
        let shadow_present = plan.shadow.is_some();
        for (index, pass) in plan.passes.iter().enumerate() {
            let depth_ops = frame::depth_operations(&plan.passes, index);
            let begin = !shadow_present && first_color == Some(index);
            let end = last_color == Some(index);
            match pass.destination {
                RenderDestination::Surface => {
                    // Surface indisponible : les passes surface sont
                    // sautées SANS écrire leurs uniforms, les passes
                    // cible s'exécutent quand même.
                    let Some(view) = &surface_view else {
                        continue;
                    };
                    self.write_pass_uniforms(pass, &plan.environment);
                    let timestamps = self
                        .timing
                        .as_ref()
                        .and_then(|timer| timer.pass_writes(begin, end));
                    let commands =
                        self.encode_pass(view, &self.depth_view, pass, depth_ops, timestamps);
                    self.queue.submit(std::iter::once(commands));
                }
                RenderDestination::Target(target) => {
                    // Une cible est toujours disponible : ni acquisition,
                    // ni présentation — le rendu hors écran fonctionne
                    // même fenêtre minimisée.
                    self.write_pass_uniforms(pass, &plan.environment);
                    let pool_handle = crate::pool::PoolHandle {
                        index: target.index,
                        generation: target.generation,
                    };
                    let Some(record) = self.render_targets.get(pool_handle) else {
                        return Err(chaos_core::ChaosError::Graphics(String::from(
                            "render target handle is stale or already destroyed",
                        )));
                    };
                    let timestamps = self
                        .timing
                        .as_ref()
                        .and_then(|timer| timer.pass_writes(begin, end));
                    let commands = self.encode_pass(
                        &record.color_view,
                        &record.depth_view,
                        pass,
                        depth_ops,
                        timestamps,
                    );
                    self.queue.submit(std::iter::once(commands));
                }
            }
        }
        // Le chronométrage se clôt APRÈS le dernier submit borné, puis
        // les mesures des frames passées se récoltent — jamais bloquant.
        if timed && let Some(timer) = &mut self.timing {
            timer.finish_frame(&self.device, &self.queue);
        }
        if let Some(frame) = surface_frame {
            self.queue.present(frame);
        }
        if let Some(timer) = &mut self.timing {
            timer.poll(&self.device);
        }
        match surface_skip {
            Some(reason) => Ok(FrameOutcome::Skipped(reason)),
            None => Ok(FrameOutcome::Rendered),
        }
    }

    fn create_render_target(
        &mut self,
        descriptor: &RenderTargetDescriptor,
    ) -> ChaosResult<(RenderTargetHandle, TextureHandle)> {
        self.build_render_target(descriptor)
    }

    fn destroy_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()> {
        self.release_render_target(handle)
    }

    fn capabilities(&self) -> RendererCapabilities {
        self.capabilities.clone()
    }

    fn gpu_frame_time(&self) -> GpuTiming {
        match &self.timing {
            None => GpuTiming::Unavailable {
                reason: String::from("timestamp queries are not supported by the adapter"),
            },
            Some(timer) => match timer.latest_ms() {
                Some(milliseconds) => GpuTiming::Measured { milliseconds },
                None => GpuTiming::Unavailable {
                    reason: String::from("no GPU measurement resolved yet"),
                },
            },
        }
    }
}

impl WgpuBackend {
    /// Écrit les uniforms d'UNE passe (vue-projection + matrices modèle),
    /// juste avant SA soumission. Le buffer frame et les slots objets
    /// sont PARTAGÉS entre les passes : la correction repose sur un
    /// submit par passe — la timeline de queue wgpu garantit qu'une
    /// écriture stagée après le submit N s'applique après ses commandes.
    /// C'est LE contrat que tout futur backend natif devra honorer.
    fn write_pass_uniforms(&mut self, pass: &FramePass, environment: &FrameEnvironment) {
        self.uniforms.write_frame(
            &self.queue,
            &convert::frame_to_bytes(pass.view_projection, pass.camera_position, environment),
        );
        // Les batches de DEBUG consomment chacun un slot d'objet APRÈS
        // ceux des draws — leur pipeline porte le layout standard mais
        // le shader ne lit pas le groupe : l'identité suffit.
        self.uniforms
            .ensure_object_slots(&self.device, pass.draws.len() + pass.debug.len());
        for (index, draw) in pass.draws.iter().enumerate() {
            self.uniforms.write_object(
                &self.queue,
                index,
                &convert::object_to_bytes(draw.model, draw.normal),
            );
        }
        for offset in 0..pass.debug.len() {
            self.uniforms.write_object(
                &self.queue,
                pass.draws.len() + offset,
                &convert::object_to_bytes(Mat4::IDENTITY, Mat4::IDENTITY),
            );
        }
        // Les transforms des draws INSTANCIÉS et les sommets de DEBUG
        // de la passe — le même contrat write → submit que le buffer
        // frame et les slots.
        self.instances.write(
            &self.device,
            &self.queue,
            &convert::instances_to_bytes(&pass.instances),
        );
        self.debug_vertices.write(
            &self.device,
            &self.queue,
            &convert::debug_vertices_to_bytes(&pass.debug_vertices),
        );
    }

    /// Exécute la passe d'ombre du plan : la vue-projection de la
    /// LUMIÈRE est la caméra de la passe (le chemin d'uniforms
    /// existant), les casters sont encodés en profondeur seule dans la
    /// shadow map interne, un submit. Sans map configurée (un plan
    /// d'ombre sans `set_shadow` — jamais produit par le Renderer), la
    /// passe est sautée avec un warn, jamais une panique.
    fn render_shadow_pass(&mut self, shadow: &FrameShadowPass, begin: bool, end: bool) {
        let Some(view) = self.shadow_view.clone() else {
            log::warn!("shadow pass skipped: no shadow map configured on the backend");
            return;
        };
        self.uniforms.write_frame(
            &self.queue,
            &convert::frame_to_bytes(
                shadow.view_projection,
                chaos_core::math::Vec3::ZERO,
                &FrameEnvironment::default(),
            ),
        );
        self.uniforms
            .ensure_object_slots(&self.device, shadow.draws.len());
        for (index, draw) in shadow.draws.iter().enumerate() {
            self.uniforms.write_object(
                &self.queue,
                index,
                &convert::object_to_bytes(draw.model, draw.normal),
            );
        }
        // Les casters instanciés : mêmes transforms, même contrat
        // write → submit — la passe d'ombre est une passe comme les
        // autres pour l'instance buffer.
        self.instances.write(
            &self.device,
            &self.queue,
            &convert::instances_to_bytes(&shadow.instances),
        );
        let timestamps = self
            .timing
            .as_ref()
            .and_then(|timer| timer.pass_writes(begin, end));
        let commands = self.encode_shadow_pass(&view, shadow, timestamps);
        self.queue.submit(std::iter::once(commands));
    }
}
