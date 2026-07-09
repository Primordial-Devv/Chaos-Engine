use chaos_core::{ChaosError, ChaosResult};

use crate::registry::AssetKind;

/// Données de texture neutres — RGBA8, rangées serrées, origine en haut à
/// gauche (convention verrouillée dans `docs/architecture/math-conventions.md`).
/// L'interprétation sRGB/linéaire appartient au consommateur : le choix du
/// format se fait au descripteur du renderer, jamais ici.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl TextureData {
    /// Cohérence sémantique : dimensions non nulles, pixels exactement
    /// aux dimensions (largeur × hauteur × 4 octets RGBA).
    pub fn validate(&self, name: &str) -> ChaosResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(ChaosError::Asset(format!(
                "texture '{name}' has zero dimensions ({}x{})",
                self.width, self.height
            )));
        }
        let expected = self.width as usize * self.height as usize * 4;
        if self.pixels.len() != expected {
            return Err(ChaosError::Asset(format!(
                "texture '{name}' expects {expected} pixel bytes ({}x{} RGBA), got {}",
                self.width,
                self.height,
                self.pixels.len()
            )));
        }
        Ok(())
    }
}

/// Données de mesh neutres — repère main droite, +Y haut, -Z avant (le
/// glTF natif partage les conventions du moteur : zéro conversion de
/// repère), UV origine en haut à gauche. `uvs.len() == positions.len()`
/// (zéros si absentes) ; indices en u32 — la contrainte u16 du renderer
/// appartient au consommateur, pas à l'import.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Cohérence sémantique : positions non vides et finies (NaN/infini
    /// rejetés, UV comprises), UV appariées aux positions, indices non
    /// vides, multiples de 3 (triangles) et tous dans les bornes — jamais
    /// de lecture hors limites côté GPU.
    pub fn validate(&self, name: &str) -> ChaosResult<()> {
        if self.positions.is_empty() {
            return Err(ChaosError::Asset(format!("mesh '{name}' has no positions")));
        }
        if !self
            .positions
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has non-finite positions (NaN or infinity)"
            )));
        }
        if !self.uvs.iter().flatten().all(|value| value.is_finite()) {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has non-finite UVs (NaN or infinity)"
            )));
        }
        if self.uvs.len() != self.positions.len() {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has {} UVs for {} positions",
                self.uvs.len(),
                self.positions.len()
            )));
        }
        if self.indices.is_empty() {
            return Err(ChaosError::Asset(format!("mesh '{name}' has no indices")));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has {} indices — not a whole number of triangles",
                self.indices.len()
            )));
        }
        let vertex_count = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        if let Some(out_of_bounds) = self.indices.iter().find(|index| **index >= vertex_count) {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has an out-of-bounds index ({out_of_bounds} for {vertex_count} vertices)"
            )));
        }
        Ok(())
    }
}

/// Ressource préparée par un importeur — le vocabulaire neutre que les
/// consommateurs cousent vers leurs descripteurs (le renderer ne voit
/// jamais un format de fichier). L'enum grandit avec les kinds : Material
/// arrivera avec sa sous-phase, puis animations, audio, scripts et scènes
/// avec leurs sous-systèmes.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportedAsset {
    Texture(TextureData),
    Mesh(MeshData),
    Shader(String),
}

impl ImportedAsset {
    /// La porte de validation sémantique du pipeline — appliquée par le
    /// Manager à TOUT import (builtin ou enregistré) : aucune donnée
    /// incohérente n'atteint les consommateurs. `Shader` passe : la
    /// validation WGSL appartient à la frontière renderer (naga en CI pour
    /// les builtins, error scope à la création) — décision documentée, pas
    /// de dépendance dupliquée dans le pipeline.
    pub fn validate(&self, name: &str) -> ChaosResult<()> {
        match self {
            Self::Texture(data) => data.validate(name),
            Self::Mesh(data) => data.validate(name),
            Self::Shader(_) => Ok(()),
        }
    }
}

/// Un importeur décode les octets d'un kind pour une famille d'extensions.
/// L'extensibilité du pipeline tient à ce contrat : enregistrer de nouveaux
/// importeurs (formats supplémentaires, kinds futurs, contenu moddé) via
/// `AssetManager::register_importer`.
pub trait AssetImporter {
    fn kind(&self) -> AssetKind;

    /// Extensions de fichier prises en charge, comparées en minuscules.
    fn extensions(&self) -> &[&str];

    fn import(&self, name: &str, bytes: &[u8]) -> ChaosResult<ImportedAsset>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sane_texture() -> TextureData {
        TextureData {
            width: 2,
            height: 1,
            pixels: vec![0; 8],
        }
    }

    fn sane_mesh() -> MeshData {
        MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn validate_accepts_sane_data() {
        assert!(sane_texture().validate("t").is_ok());
        assert!(sane_mesh().validate("m").is_ok());
        assert!(
            ImportedAsset::Shader(String::from("fn main() {}"))
                .validate("s")
                .is_ok()
        );
    }

    #[test]
    fn texture_with_zero_dimensions_is_rejected() {
        let mut texture = sane_texture();
        texture.width = 0;
        texture.pixels.clear();
        let error = texture.validate("t").unwrap_err();
        assert!(error.to_string().contains("zero dimensions"));
    }

    #[test]
    fn texture_with_mismatched_pixels_is_rejected() {
        let mut texture = sane_texture();
        texture.pixels.truncate(3);
        let error = texture.validate("t").unwrap_err();
        assert!(error.to_string().contains("expects 8 pixel bytes"));
        assert!(error.to_string().contains("got 3"));
    }

    #[test]
    fn empty_mesh_is_rejected() {
        let mesh = MeshData {
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        };
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("no positions"));
    }

    #[test]
    fn mismatched_uvs_are_rejected() {
        let mut mesh = sane_mesh();
        mesh.uvs.pop();
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("2 UVs for 3 positions"));
    }

    #[test]
    fn non_triangle_index_count_is_rejected() {
        let mut mesh = sane_mesh();
        mesh.indices.push(0);
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("whole number of triangles"));
    }

    #[test]
    fn out_of_bounds_index_is_rejected() {
        let mut mesh = sane_mesh();
        mesh.indices = vec![0, 1, 3];
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("out-of-bounds index (3"));
    }

    #[test]
    fn non_finite_positions_are_rejected() {
        let mut mesh = sane_mesh();
        mesh.positions[1][2] = f32::NAN;
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("non-finite positions"));
        let mut mesh = sane_mesh();
        mesh.uvs[0][0] = f32::INFINITY;
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("non-finite UVs"));
    }

    #[test]
    fn missing_indices_are_rejected() {
        let mut mesh = sane_mesh();
        mesh.indices.clear();
        let error = mesh.validate("m").unwrap_err();
        assert!(error.to_string().contains("no indices"));
    }
}
