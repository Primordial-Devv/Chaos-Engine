use chaos_core::{ChaosResult, Color};
use log::info;

use crate::backend::GraphicsBackend;
use crate::backend::wgpu_backend::WgpuBackend;
use crate::target::SurfaceTarget;

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
        width: u32,
        height: u32,
    ) -> ChaosResult<Self> {
        let backend = WgpuBackend::new(Box::new(target), width, height)?;
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
