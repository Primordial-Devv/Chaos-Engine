use chaos_core::{ChaosError, ChaosResult, Color};

/// Identifiant opaque d'une texture GPU. Générationnel : un handle dont la
/// ressource a été détruite est détecté, jamais résolu vers une autre texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Format des texels, dans le vocabulaire du moteur. Règle du moteur :
/// les couleurs destinées à l'affichage (albedo, UI) voyagent en sRGB,
/// les données (normal maps, roughness/metallic) en linéaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8UnormSrgb,
    R8Unorm,
    Rg8Unorm,
}

impl TextureFormat {
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb => 4,
            Self::R8Unorm => 1,
            Self::Rg8Unorm => 2,
        }
    }
}

/// Usage d'une texture : `Sampled` est uploadée puis échantillonnée par les
/// shaders ; `RenderTarget` est une cible de rendu échantillonnable —
/// vocabulaire préparé, le rendu offscreen arrivera avec ses phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureUsage {
    Sampled,
    RenderTarget,
}

/// Description d'une texture 2D ; les pixels sont uploadés à la création,
/// rangées serrées, origine en haut à gauche (convention verrouillée dans
/// `docs/architecture/math-conventions.md`). Mips, cubemaps et tableaux de
/// couches viendront avec leurs besoins réels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureDescriptor {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub usage: TextureUsage,
    pub pixels: Vec<u8>,
}

impl TextureDescriptor {
    pub fn sampled(
        label: impl Into<String>,
        width: u32,
        height: u32,
        format: TextureFormat,
        pixels: Vec<u8>,
    ) -> Self {
        Self {
            label: label.into(),
            width,
            height,
            format,
            usage: TextureUsage::Sampled,
            pixels,
        }
    }

    pub fn render_target(
        label: impl Into<String>,
        width: u32,
        height: u32,
        format: TextureFormat,
    ) -> Self {
        Self {
            label: label.into(),
            width,
            height,
            format,
            usage: TextureUsage::RenderTarget,
            pixels: Vec::new(),
        }
    }

    /// Taille attendue des pixels : largeur × hauteur × octets par texel.
    pub fn expected_byte_len(&self) -> usize {
        self.width as usize * self.height as usize * self.format.bytes_per_pixel() as usize
    }

    /// Vérifie la cohérence de la description — le point d'ancrage des
    /// règles futures (mips, cubemaps). Erreur explicite, jamais un panic ;
    /// le Renderer l'applique avant tout appel GPU, et tout constructeur de
    /// descriptions (futur asset pipeline) peut l'appliquer sans Renderer.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' has zero dimensions ({}x{})",
                self.label, self.width, self.height
            )));
        }
        match self.usage {
            TextureUsage::Sampled => {
                let expected = self.expected_byte_len();
                if self.pixels.len() != expected {
                    return Err(ChaosError::Graphics(format!(
                        "texture '{}' expects {expected} bytes of pixels ({}x{} {:?}), got {}",
                        self.label,
                        self.width,
                        self.height,
                        self.format,
                        self.pixels.len()
                    )));
                }
            }
            TextureUsage::RenderTarget => {
                if !self.pixels.is_empty() {
                    return Err(ChaosError::Graphics(format!(
                        "texture '{}' is a render target and cannot carry initial pixels",
                        self.label
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Convertit des couleurs linéaires en texels RGBA8 bruts — pour les
/// formats de DONNÉES (`Rgba8Unorm` : normal maps, masques…), que le GPU
/// lit tels quels.
pub fn rgba8_bytes_of(colors: &[Color]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(colors.len() * 4);
    for color in colors {
        bytes.extend_from_slice(&[
            channel_to_byte(color.r),
            channel_to_byte(color.g),
            channel_to_byte(color.b),
            channel_to_byte(color.a),
        ]);
    }
    bytes
}

/// Convertit des couleurs linéaires en texels RGBA8 encodés sRGB — pour
/// les formats d'AFFICHAGE (`Rgba8UnormSrgb` : albedo, UI), que le GPU
/// linéarise à l'échantillonnage. L'alpha reste linéaire (standard sRGB).
pub fn srgb8_bytes_of(colors: &[Color]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(colors.len() * 4);
    for color in colors {
        bytes.extend_from_slice(&[
            channel_to_byte(encode_srgb_channel(color.r)),
            channel_to_byte(encode_srgb_channel(color.g)),
            channel_to_byte(encode_srgb_channel(color.b)),
            channel_to_byte(color.a),
        ]);
    }
    bytes
}

/// Clamp [0, 1] puis quantification sur 8 bits ; le cast saturant garantit
/// l'absence de panic (NaN → 0).
fn channel_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Fonction de transfert sRGB de référence (IEC 61966-2-1), pas
/// l'approximation gamma 2.2.
fn encode_srgb_channel(linear: f32) -> f32 {
    let linear = linear.clamp(0.0, 1.0);
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_per_pixel_matches_each_format() {
        assert_eq!(TextureFormat::Rgba8Unorm.bytes_per_pixel(), 4);
        assert_eq!(TextureFormat::Rgba8UnormSrgb.bytes_per_pixel(), 4);
        assert_eq!(TextureFormat::R8Unorm.bytes_per_pixel(), 1);
        assert_eq!(TextureFormat::Rg8Unorm.bytes_per_pixel(), 2);
    }

    #[test]
    fn sampled_constructor_carries_usage_and_pixels() {
        let descriptor =
            TextureDescriptor::sampled("t", 2, 2, TextureFormat::Rgba8UnormSrgb, vec![255; 16]);
        assert_eq!(descriptor.usage, TextureUsage::Sampled);
        assert_eq!(descriptor.pixels.len(), 16);
        assert_eq!(descriptor.format, TextureFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn render_target_constructor_has_no_pixels() {
        let descriptor = TextureDescriptor::render_target("t", 4, 4, TextureFormat::Rgba8Unorm);
        assert_eq!(descriptor.usage, TextureUsage::RenderTarget);
        assert!(descriptor.pixels.is_empty());
    }

    #[test]
    fn expected_byte_len_multiplies_dimensions_by_texel_size() {
        let descriptor = TextureDescriptor::render_target("t", 3, 5, TextureFormat::Rg8Unorm);
        assert_eq!(descriptor.expected_byte_len(), 30);
        let wide = TextureDescriptor::render_target("t", 2, 2, TextureFormat::Rgba8Unorm);
        assert_eq!(wide.expected_byte_len(), 16);
    }

    #[test]
    fn validate_accepts_coherent_descriptions() {
        let sampled =
            TextureDescriptor::sampled("ok", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16]);
        assert!(sampled.validate().is_ok());
        let target = TextureDescriptor::render_target("rt", 4, 4, TextureFormat::Rgba8UnormSrgb);
        assert!(target.validate().is_ok());
    }

    #[test]
    fn validate_rejects_wrong_pixel_size() {
        let descriptor =
            TextureDescriptor::sampled("bad", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 3]);
        let error = descriptor.validate().unwrap_err();
        assert!(error.to_string().contains("16 bytes"));
        assert!(error.to_string().contains("got 3"));
    }

    #[test]
    fn validate_rejects_render_target_with_pixels() {
        let mut descriptor =
            TextureDescriptor::render_target("rt", 2, 2, TextureFormat::Rgba8Unorm);
        descriptor.pixels = vec![0; 16];
        let error = descriptor.validate().unwrap_err();
        assert!(error.to_string().contains("render target"));
    }

    #[test]
    fn validate_rejects_zero_dimensions() {
        let descriptor =
            TextureDescriptor::sampled("empty", 0, 4, TextureFormat::R8Unorm, Vec::new());
        let error = descriptor.validate().unwrap_err();
        assert!(error.to_string().contains("zero dimensions"));
    }

    #[test]
    fn rgba8_bytes_of_packs_four_bytes_per_color() {
        let bytes = rgba8_bytes_of(&[Color::WHITE, Color::BLACK]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[..4], &[255, 255, 255, 255]);
        assert_eq!(&bytes[4..], &[0, 0, 0, 255]);
    }

    #[test]
    fn rgba8_bytes_of_clamps_out_of_range_channels() {
        let bytes = rgba8_bytes_of(&[Color::rgba(2.0, -1.0, 0.5, 1.0)]);
        assert_eq!(bytes, vec![255, 0, 128, 255]);
    }

    #[test]
    fn srgb8_bytes_of_encodes_the_reference_transfer_function() {
        let bytes = srgb8_bytes_of(&[Color::rgb(0.0, 0.5, 1.0)]);
        assert_eq!(bytes, vec![0, 188, 255, 255]);
    }

    #[test]
    fn srgb8_bytes_of_keeps_alpha_linear() {
        let bytes = srgb8_bytes_of(&[Color::rgba(0.5, 0.5, 0.5, 0.5)]);
        assert_eq!(bytes, vec![188, 188, 188, 128]);
    }
}
