use chaos_core::{ChaosError, ChaosResult};

use crate::import::{AssetImporter, ImportedAsset, TextureData};
use crate::registry::AssetKind;

/// Importeur de textures PPM (P6 binaire, maxval 255) — le format d'image
/// standard le plus simple, décodé en std pur. Les formats compressés (PNG…)
/// arriveront avec leur sous-phase et leur décision de dépendance.
pub struct PpmImporter;

impl AssetImporter for PpmImporter {
    fn kind(&self) -> AssetKind {
        AssetKind::Texture
    }

    fn extensions(&self) -> &[&str] {
        &["ppm"]
    }

    fn import(&self, name: &str, bytes: &[u8]) -> ChaosResult<ImportedAsset> {
        parse_p6(name, bytes).map(ImportedAsset::Texture)
    }
}

fn parse_p6(name: &str, bytes: &[u8]) -> ChaosResult<TextureData> {
    let mut cursor = 0usize;
    let magic = next_token(bytes, &mut cursor);
    if magic != Some(b"P6".as_slice()) {
        return Err(ChaosError::Asset(format!(
            "texture '{name}' is not a binary PPM (P6 magic missing)"
        )));
    }
    let width = parse_header_value(name, bytes, &mut cursor, "width")?;
    let height = parse_header_value(name, bytes, &mut cursor, "height")?;
    let maxval = parse_header_value(name, bytes, &mut cursor, "maxval")?;
    if maxval != 255 {
        return Err(ChaosError::Asset(format!(
            "texture '{name}' has unsupported maxval {maxval} (only 255)"
        )));
    }
    cursor += 1;
    let expected = u64::from(width) * u64::from(height) * 3;
    let expected = usize::try_from(expected)
        .map_err(|_| ChaosError::Asset(format!("texture '{name}' dimensions are too large")))?;
    let data = bytes
        .get(cursor..cursor + expected)
        .ok_or_else(|| ChaosError::Asset(format!("texture '{name}' pixel data is truncated")))?;
    let mut pixels = Vec::with_capacity(expected / 3 * 4);
    for rgb in data.chunks_exact(3) {
        pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Ok(TextureData {
        width,
        height,
        pixels,
    })
}

/// Token d'en-tête suivant, en sautant blancs et commentaires (`#` → fin de
/// ligne). Le curseur s'arrête sur le blanc qui suit le token.
fn next_token<'bytes>(bytes: &'bytes [u8], cursor: &mut usize) -> Option<&'bytes [u8]> {
    loop {
        while bytes
            .get(*cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            *cursor += 1;
        }
        if bytes.get(*cursor) == Some(&b'#') {
            while let Some(byte) = bytes.get(*cursor) {
                *cursor += 1;
                if *byte == b'\n' {
                    break;
                }
            }
        } else {
            break;
        }
    }
    let start = *cursor;
    while bytes
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        *cursor += 1;
    }
    (start < *cursor).then(|| &bytes[start..*cursor])
}

fn parse_header_value(
    name: &str,
    bytes: &[u8],
    cursor: &mut usize,
    field: &str,
) -> ChaosResult<u32> {
    next_token(bytes, cursor)
        .and_then(|token| std::str::from_utf8(token).ok())
        .and_then(|token| token.parse::<u32>().ok())
        .ok_or_else(|| ChaosError::Asset(format!("texture '{name}' has an invalid PPM {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppm_decodes_p6_pixels_to_rgba() {
        let bytes = [b"P6\n2 1\n255\n".as_slice(), &[255, 0, 0, 0, 255, 0]].concat();
        let ImportedAsset::Texture(data) = PpmImporter.import("t", &bytes).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(data.width, 2);
        assert_eq!(data.height, 1);
        assert_eq!(data.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn ppm_header_comments_are_skipped() {
        let bytes = [
            b"P6\n# damier de test\n1 # largeur\n1\n255\n".as_slice(),
            &[10, 20, 30],
        ]
        .concat();
        let ImportedAsset::Texture(data) = PpmImporter.import("t", &bytes).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!((data.width, data.height), (1, 1));
        assert_eq!(data.pixels, vec![10, 20, 30, 255]);
    }

    #[test]
    fn ppm_rejects_a_bad_magic() {
        let error = PpmImporter.import("t", b"P3\n1 1\n255\n").unwrap_err();
        assert!(error.to_string().contains("P6 magic"));
    }

    #[test]
    fn ppm_rejects_truncated_pixel_data() {
        let bytes = [b"P6\n2 2\n255\n".as_slice(), &[1, 2, 3]].concat();
        let error = PpmImporter.import("t", &bytes).unwrap_err();
        assert!(error.to_string().contains("truncated"));
    }

    #[test]
    fn ppm_rejects_unsupported_maxval() {
        let bytes = [b"P6\n1 1\n65535\n".as_slice(), &[0, 0, 0, 0, 0, 0]].concat();
        let error = PpmImporter.import("t", &bytes).unwrap_err();
        assert!(error.to_string().contains("maxval"));
    }
}
