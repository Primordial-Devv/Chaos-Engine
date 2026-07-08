use chaos_core::Time;

/// Vue du moteur offerte aux subsystems pendant leurs hooks.
/// Portera plus tard les ressources partagées (fenêtre, assets, etc.).
#[derive(Debug, Default)]
pub struct EngineContext {
    time: Time,
    exit_requested: bool,
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

    pub(crate) fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
