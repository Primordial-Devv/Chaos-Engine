//! La couture assets → renderer : le pont entre les données neutres du
//! pipeline (`chaos_assets`) et les descripteurs du renderer — seul
//! `chaos_engine` voit les deux mondes (le patron raw-window-handle).
//! Le renderer reste sans aucune logique de format ; le pipeline reste
//! sans aucune dépendance au renderer.

use chaos_assets::{MeshData, TextureData};
use chaos_core::{ChaosError, ChaosResult};
use chaos_renderer::{TextureDescriptor, TextureFormat, TexturedGeometry, TexturedVertex};

pub use chaos_assets::{
    AssetEntry, AssetImporter, AssetKind, AssetManager, AssetSource, AssetState, ImportedAsset,
};
pub use chaos_core::AssetId;

/// Données de texture neutres → descripteur du renderer. Le choix
/// sRGB/linéaire appartient à l'appelant : couleurs destinées à l'affichage
/// en sRGB, données (normal maps…) en linéaire — la règle établie.
pub fn texture_descriptor(
    label: impl Into<String>,
    data: &TextureData,
    format: TextureFormat,
) -> TextureDescriptor {
    TextureDescriptor::sampled(label, data.width, data.height, format, data.pixels.clone())
}

/// Données de mesh neutres → géométrie texturée du renderer. C'est ici que
/// vit la limite u16 du renderer : un mesh de plus de 65 536 sommets est
/// refusé avec une erreur explicite (les indices 32 bits viendront avec
/// leur besoin) — les indices, déjà validés par la porte du pipeline
/// (tous < positions.len()), se convertissent ensuite sans risque.
pub fn textured_geometry(name: &str, data: &MeshData) -> ChaosResult<TexturedGeometry> {
    const MAX_VERTICES: usize = u16::MAX as usize + 1;
    if data.positions.len() > MAX_VERTICES {
        return Err(ChaosError::Asset(format!(
            "mesh '{name}' has {} vertices — exceeds the u16 index limit ({MAX_VERTICES})",
            data.positions.len()
        )));
    }
    let vertices = data
        .positions
        .iter()
        .zip(&data.uvs)
        .map(|(position, uv)| TexturedVertex {
            position: *position,
            uv: *uv,
        })
        .collect();
    let indices = data
        .indices
        .iter()
        .map(|index| u16::try_from(*index).unwrap_or(u16::MAX))
        .collect();
    Ok(TexturedGeometry { vertices, indices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaos_renderer::TextureUsage;

    fn mesh_data() -> MeshData {
        MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn texture_descriptor_maps_the_neutral_data() {
        let data = TextureData {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let descriptor = texture_descriptor("demo.checker", &data, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(descriptor.label, "demo.checker");
        assert_eq!((descriptor.width, descriptor.height), (2, 1));
        assert_eq!(descriptor.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(descriptor.usage, TextureUsage::Sampled);
        assert_eq!(descriptor.pixels, data.pixels);
        assert!(descriptor.validate().is_ok());
    }

    #[test]
    fn textured_geometry_maps_vertices_and_indices() {
        let geometry = textured_geometry("m", &mesh_data()).unwrap();
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(geometry.vertices[1].uv, [1.0, 0.0]);
        assert_eq!(geometry.indices, vec![0, 1, 2]);
        assert_eq!(geometry.element_count(), 3);
    }

    #[test]
    fn textured_geometry_rejects_meshes_beyond_the_u16_limit() {
        let count = usize::from(u16::MAX) + 2;
        let data = MeshData {
            positions: vec![[0.0, 0.0, 0.0]; count],
            uvs: vec![[0.0, 0.0]; count],
            indices: vec![0, 1, 2],
        };
        let error = textured_geometry("huge", &data).unwrap_err();
        assert!(error.to_string().contains("u16 index limit"));
    }

    #[test]
    fn textured_geometry_preserves_uv_pairing() {
        let mut data = mesh_data();
        data.uvs[2] = [0.25, 0.75];
        let geometry = textured_geometry("m", &data).unwrap();
        assert_eq!(geometry.vertices[2].uv, [0.25, 0.75]);
    }
}
