use chaos_window::WindowConfig;

/// Configuration de démarrage du moteur.
///
/// `frame_limit` arrête proprement le moteur après N frames (tests, CI, soak).
/// `target_fps` borne la cadence de la boucle tant qu'aucun renderer ne fournit
/// de vsync ; `None` laisse la boucle libre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    pub app_name: String,
    pub window: WindowConfig,
    pub frame_limit: Option<u64>,
    pub target_fps: Option<u32>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            app_name: String::from("Chaos Engine"),
            window: WindowConfig::default(),
            frame_limit: None,
            target_fps: Some(60),
        }
    }
}
