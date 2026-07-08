use chaos_core::{ChaosResult, Event};

use crate::context::EngineContext;

/// Point d'accroche des systèmes du moteur (renderer, scènes, ECS, physique,
/// audio, réseau, runtime…).
///
/// Les subsystems sont initialisés dans leur ordre d'enregistrement et arrêtés
/// en ordre inverse. Chaque hook reçoit le contexte du moteur.
pub trait Subsystem {
    fn name(&self) -> &str;

    fn init(&mut self, _context: &mut EngineContext) -> ChaosResult<()> {
        Ok(())
    }

    fn on_event(&mut self, _event: &Event, _context: &mut EngineContext) {}

    fn update(&mut self, _context: &mut EngineContext) {}

    fn render(&mut self, _context: &mut EngineContext) {}

    fn shutdown(&mut self, _context: &mut EngineContext) {}
}
