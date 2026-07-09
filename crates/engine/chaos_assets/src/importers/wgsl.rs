use chaos_core::{ChaosError, ChaosResult};

use crate::import::{AssetImporter, ImportedAsset};
use crate::registry::AssetKind;

/// Importeur de shaders WGSL : le texte validé UTF-8, prêt pour la
/// `ShaderLibrary` du renderer.
pub struct WgslImporter;

impl AssetImporter for WgslImporter {
    fn kind(&self) -> AssetKind {
        AssetKind::Shader
    }

    fn extensions(&self) -> &[&str] {
        &["wgsl"]
    }

    fn import(&self, name: &str, bytes: &[u8]) -> ChaosResult<ImportedAsset> {
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(ImportedAsset::Shader(String::from(text))),
            Err(utf8_error) => Err(ChaosError::Asset(format!(
                "shader '{name}' is not valid UTF-8: {utf8_error}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_imports_valid_text() {
        let ImportedAsset::Shader(text) = WgslImporter.import("s", b"fn main() {}").unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "fn main() {}");
    }

    #[test]
    fn wgsl_rejects_invalid_utf8() {
        let error = WgslImporter.import("s", &[0xff, 0xfe, 0x00]).unwrap_err();
        assert!(error.to_string().contains("UTF-8"));
    }
}
