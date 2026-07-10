use chaos_assets::AssetManager;
use chaos_core::{ChaosResult, Time};
use chaos_ecs::{Schedule, World};
use chaos_renderer::Renderer;

/// Vue du moteur offerte aux subsystems pendant leurs hooks.
/// Porte les services partagés : le renderer (`None` hors fenêtre — tests),
/// l'Asset Manager (toujours présent — aucune dépendance GPU), l'unique
/// point d'entrée des ressources, et l'ECS — le World (les données du
/// monde) et le Schedule (l'ordonnancement des systèmes) entrent par la
/// même couture que les autres services.
#[derive(Default)]
pub struct EngineContext {
    time: Time,
    exit_requested: bool,
    renderer: Option<Renderer>,
    assets: AssetManager,
    world: World,
    schedule: Schedule,
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

    /// Le monde ECS du moteur : entités, composants, ressources et
    /// messages, partagés par les systèmes et les subsystems.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// L'ordonnancement des systèmes ECS — les subsystems y enregistrent
    /// leurs systèmes pendant `init` (stage `stages::UPDATE`).
    pub fn schedule_mut(&mut self) -> &mut Schedule {
        &mut self.schedule
    }

    /// Le tick ECS de la frame : la ressource `Time` rafraîchie, puis le
    /// schedule sur le monde — avant les updates des subsystems, qui
    /// lisent l'état frais.
    pub(crate) fn tick_world(&mut self, time: Time) -> ChaosResult<()> {
        self.world.insert_resource(time);
        self.schedule.run(&mut self.world)
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
