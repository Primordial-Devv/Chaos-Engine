use crate::resources::TextureHandle;

/// Description de l'environnement de la scène : une cubemap (HDR
/// recommandé — `Rgba16Float`), son intensité et le ciel en fond de
/// scène. UN environnement actif à la fois — un état PERSISTANT du
/// Renderer (comme l'ambiante), que `clear_draws` ne touche pas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvironmentDescriptor {
    /// La cubemap d'environnement — kind `Cube` exigé (validé).
    pub cubemap: TextureHandle,
    /// L'intensité de la contribution environnementale (IBL et ciel).
    pub intensity: f32,
    /// Le ciel est-il dessiné en fond des passes `Clear` ?
    pub sky: bool,
}

impl EnvironmentDescriptor {
    /// Descripteur aux défauts du moteur : intensité 1, ciel dessiné.
    pub fn new(cubemap: TextureHandle) -> Self {
        Self {
            cubemap,
            intensity: 1.0,
            sky: true,
        }
    }

    /// Règle l'intensité de la contribution environnementale.
    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Active ou coupe le ciel en fond de scène.
    pub fn with_sky(mut self, sky: bool) -> Self {
        self.sky = sky;
        self
    }
}

/// L'inspection de l'environnement actif — le miroir lisible de l'état,
/// pour les outils (éditeur, débogage).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentInfo {
    /// Le label de la cubemap active.
    pub label: String,
    /// L'intensité de la contribution environnementale.
    pub intensity: f32,
    /// Le ciel est-il dessiné ?
    pub sky: bool,
    /// Le nombre de niveaux de mips de la cubemap (la rugosité IBL les
    /// parcourt).
    pub mip_levels: u32,
}
