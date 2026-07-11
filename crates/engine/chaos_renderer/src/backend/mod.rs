use chaos_core::ChaosResult;

use crate::capabilities::RendererCapabilities;
use crate::config::RendererConfig;
use crate::diagnostics::GpuTiming;
use crate::frame::{FrameOutcome, FramePlan};
use crate::resources::{
    BufferDescriptor, BufferHandle, MaterialBindingDescriptor, MaterialBindingHandle,
    MaterialParams, PipelineDescriptor, PipelineHandle, RenderTargetDescriptor, RenderTargetHandle,
    SamplerDescriptor, SamplerHandle, ShaderSource, TextureDescriptor, TextureHandle,
};
use crate::shadow::ShadowConfig;
use crate::target::SurfaceTarget;

mod wgpu;

/// Contrat du backend graphique : le point de remplacement du renderer.
///
/// wgpu n'est que l'implémentation actuelle ; un backend maison (Vulkan,
/// DirectX 12, Metal) devra seulement honorer ce trait pour remplacer wgpu
/// sans toucher au reste du moteur.
///
/// `Send` PAR CONTRAT : la porte du futur render thread — un backend qui
/// ne peut pas quitter le main thread est refusé à la compilation (la
/// politique complète : `docs/architecture/threading.md`).
pub trait GraphicsBackend: Send {
    /// La description humaine du backend (adaptateur, API) — logs.
    fn description(&self) -> String;

    /// Reconfigure la surface à cette taille ; une dimension nulle
    /// suspend le rendu (minimisation) au lieu de reconfigurer.
    fn resize(&mut self, width: u32, height: u32);

    /// Crée un pipeline depuis un descripteur Chaos et une source déjà
    /// résolue. Les descripteurs arrivent VALIDÉS par le Renderer.
    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
        shader: &ShaderSource,
    ) -> ChaosResult<PipelineHandle>;

    /// Crée un buffer GPU, contenu uploadé à la création.
    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle>;

    /// Détruit un buffer ; un handle périmé est une erreur explicite.
    fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()>;

    /// Crée une texture (2D ou cube), pixels uploadés à la création
    /// (layout niveau-majeur). Les descripteurs arrivent VALIDÉS et les
    /// `Generate` déjà résolus en `Provided` par le Renderer.
    fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle>;

    /// Remplace les pixels du niveau 0 d'une texture 2D mono-niveau —
    /// les octets arrivent VALIDÉS par le Renderer.
    fn update_texture(&mut self, handle: TextureHandle, pixels: &[u8]) -> ChaosResult<()>;

    /// Détruit une texture ; un handle périmé est une erreur explicite.
    fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()>;

    /// Crée un sampler.
    fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle>;

    /// Détruit un sampler ; un handle périmé est une erreur explicite.
    fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()>;

    /// Crée une cible hors écran : sa COULEUR est une texture du pool
    /// (bindable), sa profondeur lui appartient. Rend le handle de la
    /// cible ET celui de sa texture couleur.
    fn create_render_target(
        &mut self,
        descriptor: &RenderTargetDescriptor,
    ) -> ChaosResult<(RenderTargetHandle, TextureHandle)>;

    /// Détruit une cible (vues + profondeur) ; sa texture couleur suit le
    /// chemin des textures. Un handle périmé est une erreur explicite.
    fn destroy_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()>;

    /// Crée le binding GPU d'un material (groupe 2 : texture + sampler +
    /// uniforms), ressources déjà résolues par le Renderer.
    fn create_material_binding(
        &mut self,
        descriptor: &MaterialBindingDescriptor,
    ) -> ChaosResult<MaterialBindingHandle>;

    /// Met à jour les paramètres d'un binding vivant EN PLACE (le buffer
    /// d'uniforms est écrit, le bind group survit — jamais de recréation).
    /// Un handle périmé est une erreur explicite.
    fn update_material_binding(
        &mut self,
        handle: MaterialBindingHandle,
        params: &MaterialParams,
    ) -> ChaosResult<()>;

    /// Détruit un binding ; un handle périmé est une erreur explicite.
    fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()>;

    /// Fixe la cubemap d'ENVIRONNEMENT de la frame (groupe 0) — `None`
    /// rebinde le cube fallback noir interne. Le handle arrive VALIDÉ
    /// (vivant, kind `Cube`) par le Renderer.
    fn set_environment(&mut self, cubemap: Option<TextureHandle>) -> ChaosResult<()>;

    /// Fixe la ressource d'OMBRE de la frame — `Some` fait posséder au
    /// backend une shadow map interne de cette résolution (recréée si
    /// elle change) bindée au groupe frame ; `None` la libère et rebinde
    /// le fallback « tout éclairé ». La configuration arrive VALIDÉE par
    /// le Renderer ; les données par frame (vue de lumière, biais,
    /// casters) voyagent par le plan (`FramePlan.shadow`).
    fn set_shadow(&mut self, config: Option<ShadowConfig>) -> ChaosResult<()>;

    /// Exécute le plan de frame et présente — ou saute proprement avec
    /// une raison nommée (`FrameOutcome::Skipped`).
    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome>;

    /// Le temps GPU de la dernière frame RÉSOLUE — `Measured` seulement
    /// si le backend possède de vraies mesures (timestamp queries) ;
    /// sinon `Unavailable` avec sa raison. JAMAIS une valeur inventée :
    /// un backend qui ne mesure pas le DIT.
    fn gpu_frame_time(&self) -> GpuTiming;

    /// Les CAPACITÉS détectées et les décisions prises — capturées à
    /// l'initialisation, STATIQUES pour la vie du backend. Le Renderer
    /// fait respecter `limits` AVANT le backend ; chaque décision est
    /// EXPLIQUÉE (le contrat de la robustesse multiplateforme).
    fn capabilities(&self) -> RendererCapabilities;
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
