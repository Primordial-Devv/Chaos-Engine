//! La couture assets → renderer : le pont entre les données neutres du
//! pipeline (`chaos_assets`) et les descripteurs du renderer — seul
//! `chaos_engine` voit les deux mondes (le patron raw-window-handle).
//! Le renderer reste sans aucune logique de format ; le pipeline reste
//! sans aucune dépendance au renderer.

use chaos_assets::{MeshData, TextureData};
use chaos_core::math::Vec3;
use chaos_core::{ChaosError, ChaosResult};
use chaos_renderer::{
    LitGeometry, LitVertex, TextureDescriptor, TextureFormat, TexturedGeometry, TexturedVertex,
};

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

/// La limite u16 du renderer : un mesh de plus de 65 536 sommets est
/// refusé avec une erreur explicite (les indices 32 bits viendront avec
/// leur besoin) — les indices, déjà validés par la porte du pipeline
/// (tous < positions.len()), se convertissent ensuite sans risque.
fn check_u16_limit(name: &str, data: &MeshData) -> ChaosResult<()> {
    const MAX_VERTICES: usize = u16::MAX as usize + 1;
    if data.positions.len() > MAX_VERTICES {
        return Err(ChaosError::Asset(format!(
            "mesh '{name}' has {} vertices — exceeds the u16 index limit ({MAX_VERTICES})",
            data.positions.len()
        )));
    }
    Ok(())
}

/// Données de mesh neutres → géométrie texturée du renderer.
pub fn textured_geometry(name: &str, data: &MeshData) -> ChaosResult<TexturedGeometry> {
    check_u16_limit(name, data)?;
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

/// Données de mesh neutres → géométrie ÉCLAIRABLE du renderer. Les
/// normales du fichier sont appariées telles quelles ; un mesh SANS
/// normales (elles sont optionnelles à l'import) les reçoit par SYNTHÈSE
/// PLATE : la normale de chaque triangle (produit vectoriel, enroulement
/// CCW — la convention d'import) est accumulée sur ses sommets puis
/// normalisée — un sommet partagé entre faces coplanaires garde la
/// normale exacte de la face. Un sommet orphelin ou dégénéré retombe
/// sur +Y.
pub fn lit_geometry(name: &str, data: &MeshData) -> ChaosResult<LitGeometry> {
    check_u16_limit(name, data)?;
    let normals: Vec<[f32; 3]> = if data.normals.is_empty() {
        synthesize_flat_normals(data)
    } else {
        data.normals.clone()
    };
    let vertices = data
        .positions
        .iter()
        .zip(&normals)
        .zip(&data.uvs)
        .map(|((position, normal), uv)| LitVertex {
            position: *position,
            normal: *normal,
            uv: *uv,
        })
        .collect();
    let indices = data
        .indices
        .iter()
        .map(|index| u16::try_from(*index).unwrap_or(u16::MAX))
        .collect();
    Ok(LitGeometry { vertices, indices })
}

fn synthesize_flat_normals(data: &MeshData) -> Vec<[f32; 3]> {
    let mut accumulated = vec![Vec3::ZERO; data.positions.len()];
    for triangle in data.indices.chunks(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let edge_ab = Vec3::from_array(data.positions[b]) - Vec3::from_array(data.positions[a]);
        let edge_ac = Vec3::from_array(data.positions[c]) - Vec3::from_array(data.positions[a]);
        let face_normal = edge_ab.cross(edge_ac);
        for index in [a, b, c] {
            accumulated[index] += face_normal;
        }
    }
    accumulated
        .into_iter()
        .map(|normal| {
            if normal.length_squared() < f32::EPSILON {
                Vec3::Y.to_array()
            } else {
                normal.normalize().to_array()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaos_renderer::TextureUsage;

    fn mesh_data() -> MeshData {
        MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: Vec::new(),
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
            normals: Vec::new(),
            uvs: vec![[0.0, 0.0]; count],
            indices: vec![0, 1, 2],
        };
        let error = textured_geometry("huge", &data).unwrap_err();
        assert!(error.to_string().contains("u16 index limit"));
        let lit_error = lit_geometry("huge", &data).unwrap_err();
        assert!(lit_error.to_string().contains("u16 index limit"));
    }

    #[test]
    fn textured_geometry_preserves_uv_pairing() {
        let mut data = mesh_data();
        data.uvs[2] = [0.25, 0.75];
        let geometry = textured_geometry("m", &data).unwrap();
        assert_eq!(geometry.vertices[2].uv, [0.25, 0.75]);
    }

    #[test]
    fn lit_geometry_pairs_the_file_normals() {
        let mut data = mesh_data();
        data.normals = vec![[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        let geometry = lit_geometry("m", &data).unwrap();
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(geometry.vertices[1].normal, [0.0, 1.0, 0.0]);
        assert_eq!(geometry.vertices[1].uv, [1.0, 0.0]);
        assert_eq!(geometry.indices, vec![0, 1, 2]);
    }

    #[test]
    fn lit_geometry_synthesizes_flat_normals_when_absent() {
        // Le quad du sol réel (floor.glb) : plan XY, deux triangles CCW —
        // la synthèse doit rendre EXACTEMENT +Z sur les quatre sommets.
        let data = MeshData {
            positions: vec![
                [-0.5, -0.5, 0.0],
                [0.5, -0.5, 0.0],
                [0.5, 0.5, 0.0],
                [-0.5, 0.5, 0.0],
            ],
            normals: Vec::new(),
            uvs: vec![[0.0, 0.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        let geometry = lit_geometry("floor", &data).unwrap();
        for vertex in &geometry.vertices {
            assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn a_degenerate_vertex_falls_back_to_up() {
        // Un triangle d'aire nulle n'a pas de normale : le sommet retombe
        // sur +Y au lieu de produire des NaN.
        let data = MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            normals: Vec::new(),
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        };
        let geometry = lit_geometry("degenerate", &data).unwrap();
        for vertex in &geometry.vertices {
            assert_eq!(vertex.normal, [0.0, 1.0, 0.0]);
        }
    }
}
