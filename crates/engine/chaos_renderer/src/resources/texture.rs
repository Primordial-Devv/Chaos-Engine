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
    /// RGBA 8 bits par canal, linéaire — les DONNÉES (masques, packés).
    Rgba8Unorm,
    /// RGBA 8 bits par canal, sRGB — les COULEURS d'affichage (albedo).
    Rgba8UnormSrgb,
    /// Un canal 8 bits, linéaire (roughness, masques simples).
    R8Unorm,
    /// Deux canaux 8 bits, linéaires (normal maps 2 canaux, RG packé).
    Rg8Unorm,
    /// RGBA demi-flottants (16 bits/canal) — le HDR des environnements
    /// et des rendus avancés ; filtrable partout. `Rgba32Float` est
    /// ABSENT en V1 (non filtrable sans feature backend — limitation
    /// documentée, pas un piège).
    Rgba16Float,
}

impl TextureFormat {
    /// Le nombre d'octets par texel du format.
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb => 4,
            Self::R8Unorm => 1,
            Self::Rg8Unorm => 2,
            Self::Rgba16Float => 8,
        }
    }
}

/// La forme d'une texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextureKind {
    /// Une texture 2D classique — le défaut.
    #[default]
    D2,
    /// Un cubemap : 6 faces CARRÉES, ordonnées +X, -X, +Y, -Y, +Z, -Z
    /// (l'ordre des couches wgpu). Non échantillonnable par les
    /// materials en V1 — la passe environnement l'exploitera.
    Cube,
}

/// La politique de mipmaps d'une texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextureMips {
    /// Un seul niveau — le défaut.
    #[default]
    None,
    /// N niveaux FOURNIS dans `pixels` (layout niveau-majeur : chaque
    /// niveau aux dimensions divisées par deux — min 1 —, toutes les
    /// couches du niveau, rangées serrées). Validés à l'octet près.
    Provided(u32),
    /// La chaîne complète est GÉNÉRÉE par le renderer (box filter CPU) à
    /// partir du niveau 0 fourni — formats RGBA 8 bits et `Rgba16Float`,
    /// textures 2D et cubemaps (chaque face filtrée INDÉPENDAMMENT — des
    /// coutures inter-faces peuvent apparaître aux derniers niveaux,
    /// limitation V1 documentée) ; résolue en `Provided` AVANT le backend.
    Generate,
}

/// Les dimensions d'un niveau de mip (divisions par deux, minimum 1).
pub fn mip_dimensions(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// Le nombre de niveaux de la chaîne de mips COMPLÈTE pour ces dimensions.
pub fn max_mip_levels(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// Usage d'une texture : `Sampled` est uploadée puis échantillonnée par les
/// shaders ; `RenderTarget` est une cible de rendu échantillonnable —
/// vocabulaire préparé, le rendu offscreen arrivera avec ses phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureUsage {
    /// Uploadée à la création puis échantillonnée par les shaders.
    Sampled,
    /// Cible de rendu échantillonnable (rendu offscreen).
    RenderTarget,
}

/// Description d'une texture (2D ou cubemap) ; les pixels sont uploadés à
/// la création, rangées serrées, origine en haut à gauche (convention
/// verrouillée dans `docs/architecture/math-conventions.md`). Les tableaux
/// de couches généralisés et les textures 3D viendront avec leurs besoins
/// réels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureDescriptor {
    /// Le label de diagnostic.
    pub label: String,
    /// La largeur en texels — jamais zéro (validé).
    pub width: u32,
    /// La hauteur en texels — jamais zéro (validé).
    pub height: u32,
    /// Le format des texels.
    pub format: TextureFormat,
    /// L'usage de la texture.
    pub usage: TextureUsage,
    /// La forme (2D ou cubemap).
    pub kind: TextureKind,
    /// La politique de mipmaps.
    pub mips: TextureMips,
    /// Les pixels uploadés à la création — layout NIVEAU-MAJEUR : pour
    /// chaque niveau de mip, toutes les couches (6 faces d'un cube),
    /// rangées serrées, origine en haut à gauche. Vides pour une cible
    /// de rendu (validé à l'octet près).
    pub pixels: Vec<u8>,
}

impl TextureDescriptor {
    /// Descripteur d'une texture 2D échantillonnée, pixels fournis, sans
    /// mips.
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
            kind: TextureKind::D2,
            mips: TextureMips::None,
            pixels,
        }
    }

    /// Descripteur d'une cible de rendu échantillonnable, sans pixels.
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
            kind: TextureKind::D2,
            mips: TextureMips::None,
            pixels: Vec::new(),
        }
    }

    /// Descripteur d'un cubemap échantillonné : 6 faces carrées de côté
    /// `size`, pixels au layout niveau-majeur (les 6 faces du niveau 0,
    /// puis celles du niveau 1 si mips fournis, …).
    pub fn cube(
        label: impl Into<String>,
        size: u32,
        format: TextureFormat,
        pixels: Vec<u8>,
    ) -> Self {
        Self {
            label: label.into(),
            width: size,
            height: size,
            format,
            usage: TextureUsage::Sampled,
            kind: TextureKind::Cube,
            mips: TextureMips::None,
            pixels,
        }
    }

    /// Fixe la politique de mipmaps.
    pub fn with_mips(mut self, mips: TextureMips) -> Self {
        self.mips = mips;
        self
    }

    /// Le nombre de niveaux de mips effectif du descripteur.
    pub fn mip_level_count(&self) -> u32 {
        match self.mips {
            TextureMips::None => 1,
            TextureMips::Provided(levels) => levels,
            TextureMips::Generate => max_mip_levels(self.width, self.height),
        }
    }

    /// Le nombre de couches (1 en 2D, 6 pour un cubemap).
    pub fn layer_count(&self) -> u32 {
        match self.kind {
            TextureKind::D2 => 1,
            TextureKind::Cube => 6,
        }
    }

    /// Taille attendue des pixels du NIVEAU 0, une couche : largeur ×
    /// hauteur × octets par texel.
    pub fn expected_byte_len(&self) -> usize {
        self.width as usize * self.height as usize * self.format.bytes_per_pixel() as usize
    }

    /// Taille TOTALE attendue de `pixels` : tous les niveaux, toutes les
    /// couches (le layout niveau-majeur).
    pub fn expected_total_byte_len(&self) -> usize {
        let mut total = 0usize;
        for level in 0..self.mip_level_count() {
            let (width, height) = mip_dimensions(self.width, self.height, level);
            total += width as usize
                * height as usize
                * self.format.bytes_per_pixel() as usize
                * self.layer_count() as usize;
        }
        total
    }

    /// Vérifie la cohérence de la description. Erreur explicite, jamais
    /// un panic ; le Renderer l'applique avant tout appel GPU, et tout
    /// constructeur de descriptions (futur asset pipeline) peut
    /// l'appliquer sans Renderer.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' has zero dimensions ({}x{})",
                self.label, self.width, self.height
            )));
        }
        if self.kind == TextureKind::Cube && self.width != self.height {
            return Err(ChaosError::Graphics(format!(
                "cubemap '{}' must have square faces, got {}x{}",
                self.label, self.width, self.height
            )));
        }
        if let TextureMips::Provided(levels) = self.mips {
            let max = max_mip_levels(self.width, self.height);
            if levels == 0 || levels > max {
                return Err(ChaosError::Graphics(format!(
                    "texture '{}' declares {levels} mip level(s), the full chain for {}x{} holds {max}",
                    self.label, self.width, self.height
                )));
            }
        }
        if self.mips == TextureMips::Generate
            && !matches!(
                self.format,
                TextureFormat::Rgba8Unorm
                    | TextureFormat::Rgba8UnormSrgb
                    | TextureFormat::Rgba16Float
            )
        {
            return Err(ChaosError::Graphics(format!(
                "texture '{}': mip generation supports 8-bit RGBA and Rgba16Float formats in V1, got {:?}",
                self.label, self.format
            )));
        }
        match self.usage {
            TextureUsage::Sampled => {
                let expected = match self.mips {
                    TextureMips::Generate => self.expected_byte_len() * self.layer_count() as usize,
                    _ => self.expected_total_byte_len(),
                };
                if self.pixels.len() != expected {
                    return Err(ChaosError::Graphics(format!(
                        "texture '{}' expects {expected} bytes of pixels ({}x{} {:?}, {} level(s), {} layer(s)), got {}",
                        self.label,
                        self.width,
                        self.height,
                        self.format,
                        match self.mips {
                            TextureMips::Generate => 1,
                            _ => self.mip_level_count(),
                        },
                        self.layer_count(),
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
                if self.kind != TextureKind::D2 {
                    return Err(ChaosError::Graphics(format!(
                        "texture '{}': cube render targets are not supported in V1",
                        self.label
                    )));
                }
                if self.mips != TextureMips::None {
                    return Err(ChaosError::Graphics(format!(
                        "texture '{}': mipmapped render targets are not supported in V1",
                        self.label
                    )));
                }
            }
        }
        Ok(())
    }

    /// Résout `Generate` en `Provided` : la chaîne complète est calculée
    /// par box filter CPU depuis le niveau 0 (formats RGBA 8 bits ou
    /// `Rgba16Float` — garanti par `validate`). Chaque couche (les 6 faces
    /// d'un cubemap) est filtrée INDÉPENDAMMENT, dans l'ordre du layout
    /// niveau-majeur. Les autres politiques passent inchangées.
    pub(crate) fn resolved_mips(&self) -> Self {
        if self.mips != TextureMips::Generate {
            return self.clone();
        }
        let levels = max_mip_levels(self.width, self.height);
        let layers = self.layer_count() as usize;
        let mut pixels = self.pixels.clone();
        let mut previous = self.pixels.clone();
        let (mut width, mut height) = (self.width, self.height);
        for _ in 1..levels {
            let face_bytes = previous.len() / layers;
            let mut next = Vec::new();
            for layer in 0..layers {
                let face = &previous[layer * face_bytes..(layer + 1) * face_bytes];
                let reduced = match self.format {
                    TextureFormat::Rgba16Float => downsample_rgba16f(face, width, height),
                    _ => downsample_rgba8(face, width, height),
                };
                next.extend_from_slice(&reduced);
            }
            pixels.extend_from_slice(&next);
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            previous = next;
        }
        let mut resolved = self.clone();
        resolved.mips = TextureMips::Provided(levels);
        resolved.pixels = pixels;
        resolved
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

/// Les textures builtin du moteur — les fallbacks ADAPTÉS AUX USAGES,
/// créés au premier usage, partagés et PROTÉGÉS (indestructibles,
/// non modifiables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTexture {
    /// Blanc 1×1 (`chaos.white`) — l'albédo neutre, le fallback des
    /// materials sans texture.
    White,
    /// Noir 1×1 (`chaos.black`) — masques et émissifs neutres.
    Black,
    /// Normale plate 1×1 (`chaos.normal_flat`, 128/128/255 linéaire) —
    /// le fallback des normal maps du futur PBR.
    FlatNormal,
}

impl BuiltinTexture {
    /// Le descripteur complet de la texture builtin.
    pub fn descriptor(self) -> TextureDescriptor {
        match self {
            Self::White => TextureDescriptor::sampled(
                "chaos.white",
                1,
                1,
                TextureFormat::Rgba8Unorm,
                vec![255, 255, 255, 255],
            ),
            Self::Black => TextureDescriptor::sampled(
                "chaos.black",
                1,
                1,
                TextureFormat::Rgba8Unorm,
                vec![0, 0, 0, 255],
            ),
            Self::FlatNormal => TextureDescriptor::sampled(
                "chaos.normal_flat",
                1,
                1,
                TextureFormat::Rgba8Unorm,
                vec![128, 128, 255, 255],
            ),
        }
    }
}

/// Convertit des flottants en texels RGBA demi-flottants (`Rgba16Float`)
/// — 4 valeurs par texel, conversion IEEE 754 binary16 maison (mantisse
/// tronquée, dénormaux arrondis à zéro — suffisant pour des données HDR ;
/// zéro dépendance).
pub fn rgba16f_bytes_of(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 2);
    for value in values {
        bytes.extend_from_slice(&f32_to_f16_bits(*value).to_ne_bytes());
    }
    bytes
}

fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007F_FFFF;
    if exponent == 0xFF {
        return sign | 0x7C00 | u16::from(mantissa != 0);
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1F {
        return sign | 0x7C00;
    }
    if half_exponent <= 0 {
        return sign;
    }
    sign | ((half_exponent as u16) << 10) | ((mantissa >> 13) as u16)
}

fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x03FF);
    let value = match exponent {
        0 => sign,
        0x1F => sign | 0x7F80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(value)
}

/// Réduit un niveau `Rgba16Float` de moitié par box filter 2×2 — décodage
/// f16 → moyenne en f32 (les valeurs HDR au-delà de 1 se moyennent sans
/// écrêtage) → ré-encodage. Les dénormaux f16 sont lus comme zéro, en
/// cohérence avec l'encodeur maison.
fn downsample_rgba16f(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let next_width = (width / 2).max(1) as usize;
    let next_height = (height / 2).max(1) as usize;
    let width = width as usize;
    let read = |x: usize, y: usize, channel: usize| -> f32 {
        let offset = ((y * width + x) * 4 + channel) * 2;
        f16_bits_to_f32(u16::from_ne_bytes([pixels[offset], pixels[offset + 1]]))
    };
    let mut next = Vec::with_capacity(next_width * next_height * 8);
    for y in 0..next_height {
        for x in 0..next_width {
            let x0 = (x * 2).min(width.saturating_sub(1));
            let x1 = (x * 2 + 1).min(width.saturating_sub(1));
            let y0 = y * 2;
            let y1 = (y * 2 + 1).min((height as usize).saturating_sub(1));
            for channel in 0..4 {
                let sum = read(x0, y0, channel)
                    + read(x1, y0, channel)
                    + read(x0, y1, channel)
                    + read(x1, y1, channel);
                next.extend_from_slice(&f32_to_f16_bits(sum / 4.0).to_ne_bytes());
            }
        }
    }
    next
}

/// Réduit un niveau RGBA8 de moitié par box filter 2×2 (moyenne entière
/// par canal) ; les dimensions impaires perdent leur dernier texel
/// (division entière — le comportement standard des chaînes de mips).
fn downsample_rgba8(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let next_width = (width / 2).max(1) as usize;
    let next_height = (height / 2).max(1) as usize;
    let width = width as usize;
    let mut next = Vec::with_capacity(next_width * next_height * 4);
    for y in 0..next_height {
        for x in 0..next_width {
            let x0 = (x * 2).min(width.saturating_sub(1));
            let x1 = (x * 2 + 1).min(width.saturating_sub(1));
            let y0 = y * 2;
            let y1 = (y * 2 + 1).min((height as usize).saturating_sub(1));
            for channel in 0..4 {
                let sum = u16::from(pixels[(y0 * width + x0) * 4 + channel])
                    + u16::from(pixels[(y0 * width + x1) * 4 + channel])
                    + u16::from(pixels[(y1 * width + x0) * 4 + channel])
                    + u16::from(pixels[(y1 * width + x1) * 4 + channel]);
                next.push((sum / 4) as u8);
            }
        }
    }
    next
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
        assert_eq!(TextureFormat::Rgba16Float.bytes_per_pixel(), 8);
    }

    #[test]
    fn mip_math_is_exact() {
        assert_eq!(mip_dimensions(8, 4, 0), (8, 4));
        assert_eq!(mip_dimensions(8, 4, 1), (4, 2));
        assert_eq!(mip_dimensions(8, 4, 3), (1, 1));
        assert_eq!(max_mip_levels(8, 4), 4);
        assert_eq!(max_mip_levels(1, 1), 1);
        assert_eq!(max_mip_levels(5, 3), 3);
    }

    #[test]
    fn a_provided_mip_chain_is_validated_to_the_byte() {
        let level0 = vec![0u8; 4 * 4 * 4];
        let level1 = vec![0u8; 2 * 2 * 4];
        let level2 = vec![0u8; 4];
        let mut pixels = level0.clone();
        pixels.extend_from_slice(&level1);
        pixels.extend_from_slice(&level2);
        let descriptor = TextureDescriptor::sampled("m", 4, 4, TextureFormat::Rgba8Unorm, pixels)
            .with_mips(TextureMips::Provided(3));
        assert_eq!(descriptor.validate(), Ok(()));

        let truncated = TextureDescriptor::sampled("m", 4, 4, TextureFormat::Rgba8Unorm, level0)
            .with_mips(TextureMips::Provided(3));
        let error = truncated.validate().unwrap_err().to_string();
        assert!(error.contains("3 level(s)"));
        assert!(error.contains("got 64"));
    }

    #[test]
    fn too_many_mip_levels_are_refused() {
        let descriptor =
            TextureDescriptor::sampled("m", 4, 4, TextureFormat::Rgba8Unorm, vec![0; 64])
                .with_mips(TextureMips::Provided(5));
        let error = descriptor.validate().unwrap_err().to_string();
        assert!(error.contains("5 mip level(s)"));
        assert!(error.contains("holds 3"));
    }

    #[test]
    fn a_cubemap_must_be_square_with_six_faces() {
        let squished = TextureDescriptor {
            width: 4,
            height: 2,
            ..TextureDescriptor::cube("c", 4, TextureFormat::Rgba8Unorm, Vec::new())
        };
        assert!(
            squished
                .validate()
                .unwrap_err()
                .to_string()
                .contains("square faces")
        );

        let complete =
            TextureDescriptor::cube("c", 2, TextureFormat::Rgba8Unorm, vec![0; 2 * 2 * 4 * 6]);
        assert_eq!(complete.validate(), Ok(()));

        let missing_faces =
            TextureDescriptor::cube("c", 2, TextureFormat::Rgba8Unorm, vec![0; 2 * 2 * 4 * 5]);
        let error = missing_faces.validate().unwrap_err().to_string();
        assert!(error.contains("6 layer(s)"));
    }

    #[test]
    fn render_targets_refuse_cube_and_mips() {
        let mut cube_target =
            TextureDescriptor::render_target("t", 4, 4, TextureFormat::Rgba8Unorm);
        cube_target.kind = TextureKind::Cube;
        assert!(
            cube_target
                .validate()
                .unwrap_err()
                .to_string()
                .contains("cube render targets")
        );

        let mipped_target = TextureDescriptor::render_target("t", 4, 4, TextureFormat::Rgba8Unorm)
            .with_mips(TextureMips::Provided(2));
        assert!(
            mipped_target
                .validate()
                .unwrap_err()
                .to_string()
                .contains("mipmapped render targets")
        );
    }

    #[test]
    fn mip_generation_formats_are_bounded() {
        let single = TextureDescriptor::sampled("s", 2, 2, TextureFormat::R8Unorm, vec![0; 4])
            .with_mips(TextureMips::Generate);
        let error = single.validate().unwrap_err().to_string();
        assert!(error.contains("8-bit RGBA and Rgba16Float"));
        assert!(error.contains("R8Unorm"));

        let dual = TextureDescriptor::sampled("d", 2, 2, TextureFormat::Rg8Unorm, vec![0; 8])
            .with_mips(TextureMips::Generate);
        assert!(dual.validate().is_err());

        let cube = TextureDescriptor::cube("c", 2, TextureFormat::Rgba8Unorm, vec![0; 96])
            .with_mips(TextureMips::Generate);
        assert_eq!(cube.validate(), Ok(()));

        let hdr_cube =
            TextureDescriptor::cube("h", 2, TextureFormat::Rgba16Float, vec![0; 2 * 2 * 8 * 6])
                .with_mips(TextureMips::Generate);
        assert_eq!(hdr_cube.validate(), Ok(()));
    }

    #[test]
    fn cube_mips_generate_per_face() {
        let mut pixels = Vec::new();
        for face in 0..6u8 {
            // Chaque face porte sa propre valeur : la moyenne par face la préserve.
            pixels.extend_from_slice(&[face * 40; 2 * 2 * 4]);
        }
        let descriptor = TextureDescriptor::cube("c", 2, TextureFormat::Rgba8Unorm, pixels)
            .with_mips(TextureMips::Generate);
        assert_eq!(descriptor.validate(), Ok(()));
        let resolved = descriptor.resolved_mips();
        assert_eq!(resolved.mips, TextureMips::Provided(2));
        assert_eq!(resolved.pixels.len(), 96 + 24);
        for face in 0..6usize {
            let texel = &resolved.pixels[96 + face * 4..96 + face * 4 + 4];
            assert_eq!(texel, &[face as u8 * 40; 4]);
        }
    }

    #[test]
    fn rgba16f_mips_average_in_float() {
        let mut values = Vec::new();
        for texel in [8.0f32, 4.0, 2.0, 2.0] {
            values.extend_from_slice(&[texel; 4]);
        }
        let descriptor = TextureDescriptor::sampled(
            "h",
            2,
            2,
            TextureFormat::Rgba16Float,
            rgba16f_bytes_of(&values),
        )
        .with_mips(TextureMips::Generate);
        assert_eq!(descriptor.validate(), Ok(()));
        let resolved = descriptor.resolved_mips();
        assert_eq!(resolved.mips, TextureMips::Provided(2));
        assert_eq!(resolved.pixels.len(), 32 + 8);
        let halves: Vec<u16> = resolved.pixels[32..]
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        // (8 + 4 + 2 + 2) / 4 = 4.0 — préservé au-delà de 1, en binary16 exact.
        assert_eq!(halves, vec![0x4400; 4]);
    }

    #[test]
    fn generated_mips_average_exactly() {
        let level0: Vec<u8> = vec![
            10, 20, 30, 40, 50, 60, 70, 80, //
            90, 100, 110, 120, 130, 140, 150, 160,
        ];
        let descriptor =
            TextureDescriptor::sampled("g", 2, 2, TextureFormat::Rgba8Unorm, level0.clone())
                .with_mips(TextureMips::Generate);
        assert_eq!(descriptor.validate(), Ok(()));
        let resolved = descriptor.resolved_mips();
        assert_eq!(resolved.mips, TextureMips::Provided(2));
        assert_eq!(resolved.pixels.len(), 16 + 4);
        assert_eq!(&resolved.pixels[16..], &[70, 80, 90, 100]);
    }

    #[test]
    fn rgba16f_bytes_lock_the_half_float_conversion() {
        let bytes = rgba16f_bytes_of(&[1.0, 0.5, -2.0, 0.0]);
        assert_eq!(bytes.len(), 8);
        let halves: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(halves, vec![0x3C00, 0x3800, 0xC000, 0x0000]);
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
