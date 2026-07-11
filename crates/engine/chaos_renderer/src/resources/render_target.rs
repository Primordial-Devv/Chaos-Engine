use chaos_core::{ChaosError, ChaosResult};

use crate::resources::texture::TextureFormat;

/// Identifiant opaque d'une render target. Générationnel : un handle dont
/// la cible a été détruite (ou redimensionnée — le resize fait TOURNER le
/// handle) est détecté, jamais résolu vers une autre cible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderTargetHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Description d'une cible de rendu hors écran : une couleur
/// échantillonnable + une profondeur PROPRE (toujours incluse en V1 —
/// les pipelines du moteur attendent un depth-stencil). Les dimensions
/// sont indépendantes de la fenêtre. Multi-attachments (MRT), cibles
/// cube et profondeur optionnelle : hors périmètre V1, documenté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderTargetDescriptor {
    /// Le label de diagnostic.
    pub label: String,
    /// La largeur en pixels — jamais zéro (validé).
    pub width: u32,
    /// La hauteur en pixels — jamais zéro (validé).
    pub height: u32,
    /// Le format de l'attachment couleur — tout format échantillonnable
    /// (`Rgba16Float` = offscreen HDR). Un pipeline utilisé vers cette
    /// cible doit viser CE format (`with_color_target`).
    pub format: TextureFormat,
}

impl RenderTargetDescriptor {
    /// Descripteur aux dimensions et format donnés.
    pub fn new(label: impl Into<String>, width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            label: label.into(),
            width,
            height,
            format,
        }
    }

    /// Vérifie la cohérence de la description — erreur explicite, jamais
    /// un panic ; appliquée par le Renderer avant tout appel GPU.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(ChaosError::Graphics(format!(
                "render target '{}' has zero dimensions ({}x{})",
                self.label, self.width, self.height
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_dimensions_are_refused() {
        let flat = RenderTargetDescriptor::new("t", 0, 4, TextureFormat::Rgba8UnormSrgb);
        assert!(
            flat.validate()
                .unwrap_err()
                .to_string()
                .contains("zero dimensions")
        );
        let valid = RenderTargetDescriptor::new("t", 4, 4, TextureFormat::Rgba16Float);
        assert_eq!(valid.validate(), Ok(()));
    }
}
