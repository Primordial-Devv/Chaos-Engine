use chaos_core::{ChaosResult, Color};

pub(crate) mod wgpu_backend;

/// Contrat du backend graphique : le point de remplacement du renderer.
///
/// wgpu n'est que l'implémentation actuelle ; un backend maison (Vulkan,
/// DirectX 12, Metal) devra seulement honorer ce trait pour remplacer wgpu
/// sans toucher au reste du moteur.
pub trait GraphicsBackend {
    fn description(&self) -> String;

    fn resize(&mut self, width: u32, height: u32);

    fn render_frame(&mut self, clear_color: Color) -> ChaosResult<()>;
}
