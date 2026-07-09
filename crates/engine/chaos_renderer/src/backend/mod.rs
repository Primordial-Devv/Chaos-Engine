use chaos_core::ChaosResult;

use crate::config::RendererConfig;
use crate::frame::{FrameOutcome, FramePlan};
use crate::resources::{
    BufferDescriptor, BufferHandle, MaterialBindingDescriptor, MaterialBindingHandle,
    PipelineDescriptor, PipelineHandle, SamplerDescriptor, SamplerHandle, ShaderSource,
    TextureDescriptor, TextureHandle,
};
use crate::target::SurfaceTarget;

mod wgpu;

/// Contrat du backend graphique : le point de remplacement du renderer.
///
/// wgpu n'est que l'implémentation actuelle ; un backend maison (Vulkan,
/// DirectX 12, Metal) devra seulement honorer ce trait pour remplacer wgpu
/// sans toucher au reste du moteur.
pub trait GraphicsBackend {
    fn description(&self) -> String;

    fn resize(&mut self, width: u32, height: u32);

    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
        shader: &ShaderSource,
    ) -> ChaosResult<PipelineHandle>;

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle>;

    fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()>;

    fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle>;

    fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()>;

    fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle>;

    fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()>;

    fn create_material_binding(
        &mut self,
        descriptor: &MaterialBindingDescriptor,
    ) -> ChaosResult<MaterialBindingHandle>;

    fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()>;

    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome>;
}

/// Construit le backend graphique par défaut.
/// Unique endroit du moteur qui connaît les backends concrets : un futur
/// backend maison sera une branche de plus ici, rien d'autre ne change.
pub(crate) fn create_backend(
    target: Box<dyn SurfaceTarget>,
    config: RendererConfig,
) -> ChaosResult<Box<dyn GraphicsBackend>> {
    Ok(Box::new(wgpu::WgpuBackend::new(target, config)?))
}
