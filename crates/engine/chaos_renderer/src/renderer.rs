use chaos_core::{ChaosResult, Color};
use log::info;

use crate::backend::GraphicsBackend;
use crate::backend::wgpu_backend::WgpuBackend;
use crate::target::SurfaceTarget;

/// Paramètres d'attachement du renderer.
///
/// `vsync` synchronise la présentation sur le rafraîchissement de l'écran ;
/// désactivé, la présentation ne bloque jamais le thread appelant (la cadence
/// est alors régulée par l'hôte, ex. `target_fps` du moteur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererConfig {
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}

/// Renderer du moteur : orchestre un backend graphique interchangeable.
///
/// L'API ne parle que le vocabulaire de chaos_core ; le backend concret
/// (wgpu aujourd'hui) reste un détail d'implémentation interne.
pub struct Renderer {
    backend: Box<dyn GraphicsBackend>,
    clear_color: Color,
}

impl Renderer {
    /// Attache le renderer à une cible de présentation et initialise le GPU.
    pub fn attach(
        target: impl SurfaceTarget + 'static,
        config: RendererConfig,
    ) -> ChaosResult<Self> {
        let backend = WgpuBackend::new(Box::new(target), config)?;
        info!("renderer ready: {}", backend.description());
        Ok(Self {
            backend: Box::new(backend),
            clear_color: Color::BLACK,
        })
    }

    pub fn description(&self) -> String {
        self.backend.description()
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.backend.resize(width, height);
    }

    pub fn render_frame(&mut self) -> ChaosResult<()> {
        self.backend.render_frame(self.clear_color)
    }
}
