/// Description de la fenêtre principale demandée au système.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: String::from("Chaos Engine"),
            width: 1280,
            height: 720,
            resizable: true,
        }
    }
}
