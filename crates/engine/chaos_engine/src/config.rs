use chaos_core::Color;
use chaos_window::WindowConfig;

/// Configuration de démarrage du moteur.
///
/// `frame_limit` arrête proprement le moteur après N frames (tests, CI, soak).
/// `target_fps` fixe la cadence de la boucle (via l'attente native de l'OS,
/// jamais un sleep bloquant) ; `None` laisse la boucle libre. `vsync` active
/// la synchronisation verticale de la présentation — désactivée par défaut :
/// un present bloquant sur le main thread rend les interactions fenêtre
/// (déplacement, resize) laggy sur macOS. `clear_color` est la couleur de
/// fond présentée par le renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub app_name: String,
    pub window: WindowConfig,
    pub frame_limit: Option<u64>,
    pub target_fps: Option<u32>,
    pub vsync: bool,
    pub clear_color: Color,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            app_name: String::from("Chaos Engine"),
            window: WindowConfig::default(),
            frame_limit: None,
            target_fps: Some(60),
            vsync: false,
            clear_color: Color::rgb(0.02, 0.02, 0.03),
        }
    }
}
