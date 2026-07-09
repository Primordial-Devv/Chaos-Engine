use chaos_assets::AssetManager;
use chaos_core::Time;
use chaos_renderer::Renderer;

/// Vue du moteur offerte aux subsystems pendant leurs hooks.
/// Porte les services partagés : le renderer (`None` hors fenêtre — tests)
/// et l'Asset Manager (toujours présent — aucune dépendance GPU), l'unique
/// point d'entrée des ressources.
#[derive(Default)]
pub struct EngineContext {
    time: Time,
    exit_requested: bool,
    renderer: Option<Renderer>,
    assets: AssetManager,
}

impl EngineContext {
    pub fn time(&self) -> Time {
        self.time
    }

    /// Demande l'arrêt propre du moteur à la fin de la frame courante.
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub fn renderer(&self) -> Option<&Renderer> {
        self.renderer.as_ref()
    }

    pub fn renderer_mut(&mut self) -> Option<&mut Renderer> {
        self.renderer.as_mut()
    }

    /// L'Asset Manager du moteur — l'unique point d'entrée des ressources.
    pub fn assets(&self) -> &AssetManager {
        &self.assets
    }

    pub fn assets_mut(&mut self) -> &mut AssetManager {
        &mut self.assets
    }

    pub(crate) fn set_time(&mut self, time: Time) {
        self.time = time;
    }

    pub(crate) fn set_renderer(&mut self, renderer: Renderer) {
        self.renderer = Some(renderer);
    }

    pub(crate) fn take_renderer(&mut self) -> Option<Renderer> {
        self.renderer.take()
    }
}
