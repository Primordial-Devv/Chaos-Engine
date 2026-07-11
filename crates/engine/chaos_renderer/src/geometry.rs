use chaos_core::Color;
use chaos_core::math::Vec3;

use crate::resources::{ColorVertex, LitVertex, TexturedVertex, bytes_of_u16};

/// Géométrie côté CPU : la donnée moteur, indépendante de toute
/// représentation GPU. `indices` vide = rendu non indexé.
/// Indices en u16 pour l'instant ; u32 viendra avec les gros meshes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    /// Les sommets colorés.
    pub vertices: Vec<ColorVertex>,
    /// Les indices u16 ; vide = rendu non indexé.
    pub indices: Vec<u16>,
}

impl Geometry {
    /// Triangle non indexé, une couleur par sommet (dégradé) — pointe en
    /// haut, enroulement CCW.
    pub fn triangle(center: [f32; 3], size: f32, colors: [Color; 3]) -> Self {
        let half = size / 2.0;
        let positions = [
            [center[0], center[1] + half, center[2]],
            [center[0] - half, center[1] - half, center[2]],
            [center[0] + half, center[1] - half, center[2]],
        ];
        let vertices = positions
            .iter()
            .zip(colors.iter())
            .map(|(position, color)| ColorVertex {
                position: *position,
                color: [color.r, color.g, color.b],
            })
            .collect();
        Self {
            vertices,
            indices: Vec::new(),
        }
    }

    /// Quad indexé (4 sommets, 6 indices) dans le plan XY, couleur unie,
    /// enroulement CCW.
    pub fn quad(center: [f32; 3], width: f32, height: f32, color: Color) -> Self {
        let half_width = width / 2.0;
        let half_height = height / 2.0;
        let color = [color.r, color.g, color.b];
        let vertices = vec![
            ColorVertex {
                position: [center[0] - half_width, center[1] - half_height, center[2]],
                color,
            },
            ColorVertex {
                position: [center[0] + half_width, center[1] - half_height, center[2]],
                color,
            },
            ColorVertex {
                position: [center[0] + half_width, center[1] + half_height, center[2]],
                color,
            },
            ColorVertex {
                position: [center[0] - half_width, center[1] + half_height, center[2]],
                color,
            },
        ];
        Self {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// Cube fermé : 24 sommets (4 par face, une couleur par face), 36 indices.
    /// Faces ordonnées +X, -X, +Y, -Y, +Z, -Z ; enroulement CCW vu de
    /// l'extérieur (convention moteur, compatible back-face culling).
    pub fn cube(center: [f32; 3], size: f32, face_colors: [Color; 6]) -> Self {
        let half = size / 2.0;
        let origin = Vec3::from_array(center);
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for ((normal, u, v), color) in faces.into_iter().zip(face_colors) {
            let base = u16::try_from(vertices.len()).unwrap_or(u16::MAX);
            let corners = [
                origin + (normal - u - v) * half,
                origin + (normal + u - v) * half,
                origin + (normal + u + v) * half,
                origin + (normal - u + v) * half,
            ];
            for corner in corners {
                vertices.push(ColorVertex {
                    position: corner.to_array(),
                    color: [color.r, color.g, color.b],
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Self { vertices, indices }
    }

    /// La géométrie est-elle indexée ?
    pub fn is_indexed(&self) -> bool {
        !self.indices.is_empty()
    }

    /// Nombre d'éléments à dessiner : indices si indexé, sommets sinon.
    pub fn element_count(&self) -> u32 {
        let count = if self.is_indexed() {
            self.indices.len()
        } else {
            self.vertices.len()
        };
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Les sommets sérialisés pour un vertex buffer.
    pub fn vertex_bytes(&self) -> Vec<u8> {
        ColorVertex::bytes_of(&self.vertices)
    }

    /// Les indices sérialisés pour un index buffer.
    pub fn index_bytes(&self) -> Vec<u8> {
        bytes_of_u16(&self.indices)
    }
}

/// Géométrie CPU à sommets texturés (position + UV) — mêmes contrats que
/// `Geometry`. Les deux types seront unifiés quand l'asset pipeline imposera
/// des vertex formats arbitraires ; d'ici là, chacun reste simple.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TexturedGeometry {
    /// Les sommets texturés.
    pub vertices: Vec<TexturedVertex>,
    /// Les indices u16 ; vide = rendu non indexé.
    pub indices: Vec<u16>,
}

impl TexturedGeometry {
    /// Cube texturé fermé : 24 sommets (4 par face, UV 0..1 par face,
    /// origine en haut à gauche), 36 indices. Faces ordonnées +X, -X, +Y,
    /// -Y, +Z, -Z ; enroulement CCW vu de l'extérieur — mêmes conventions
    /// que `Geometry::cube`.
    pub fn cube(center: [f32; 3], size: f32) -> Self {
        let half = size / 2.0;
        let origin = Vec3::from_array(center);
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        let corner_uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for (normal, u, v) in faces {
            let base = u16::try_from(vertices.len()).unwrap_or(u16::MAX);
            let corners = [
                origin + (normal - u - v) * half,
                origin + (normal + u - v) * half,
                origin + (normal + u + v) * half,
                origin + (normal - u + v) * half,
            ];
            for (corner, uv) in corners.into_iter().zip(corner_uvs) {
                vertices.push(TexturedVertex {
                    position: corner.to_array(),
                    uv,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Self { vertices, indices }
    }

    /// Quad texturé : UV 0..`uv_scale` (origine en haut à gauche — un
    /// `uv_scale` > 1 répète la texture sous un sampler `Repeat`).
    pub fn quad(center: [f32; 3], width: f32, height: f32, uv_scale: f32) -> Self {
        let half_width = width / 2.0;
        let half_height = height / 2.0;
        let vertices = vec![
            TexturedVertex {
                position: [center[0] - half_width, center[1] - half_height, center[2]],
                uv: [0.0, uv_scale],
            },
            TexturedVertex {
                position: [center[0] + half_width, center[1] - half_height, center[2]],
                uv: [uv_scale, uv_scale],
            },
            TexturedVertex {
                position: [center[0] + half_width, center[1] + half_height, center[2]],
                uv: [uv_scale, 0.0],
            },
            TexturedVertex {
                position: [center[0] - half_width, center[1] + half_height, center[2]],
                uv: [0.0, 0.0],
            },
        ];
        Self {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// La géométrie est-elle indexée ?
    pub fn is_indexed(&self) -> bool {
        !self.indices.is_empty()
    }

    /// Nombre d'éléments à dessiner : indices si indexé, sommets sinon.
    pub fn element_count(&self) -> u32 {
        let count = if self.is_indexed() {
            self.indices.len()
        } else {
            self.vertices.len()
        };
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Les sommets sérialisés pour un vertex buffer.
    pub fn vertex_bytes(&self) -> Vec<u8> {
        TexturedVertex::bytes_of(&self.vertices)
    }

    /// Les indices sérialisés pour un index buffer.
    pub fn index_bytes(&self) -> Vec<u8> {
        bytes_of_u16(&self.indices)
    }
}

/// Géométrie CPU à sommets ÉCLAIRABLES (position + normale + UV) — mêmes
/// contrats que `TexturedGeometry`. Les constructeurs cessent de jeter
/// les normales de face qu'ils calculaient déjà.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LitGeometry {
    /// Les sommets éclairables.
    pub vertices: Vec<LitVertex>,
    /// Les indices u16 ; vide = rendu non indexé.
    pub indices: Vec<u16>,
}

impl LitGeometry {
    /// Cube éclairable fermé : 24 sommets (4 par face — la normale de
    /// face exige des sommets non partagés), UV 0..1 par face, 36
    /// indices. Faces ordonnées +X, -X, +Y, -Y, +Z, -Z ; enroulement CCW
    /// vu de l'extérieur — mêmes conventions que `TexturedGeometry::cube`.
    pub fn cube(center: [f32; 3], size: f32) -> Self {
        let half = size / 2.0;
        let origin = Vec3::from_array(center);
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        let corner_uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for (normal, u, v) in faces {
            let base = u16::try_from(vertices.len()).unwrap_or(u16::MAX);
            let corners = [
                origin + (normal - u - v) * half,
                origin + (normal + u - v) * half,
                origin + (normal + u + v) * half,
                origin + (normal - u + v) * half,
            ];
            for (corner, uv) in corners.into_iter().zip(corner_uvs) {
                vertices.push(LitVertex {
                    position: corner.to_array(),
                    normal: normal.to_array(),
                    uv,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Self { vertices, indices }
    }

    /// Sphère UV éclairable : `rings` anneaux de latitude × `segments`
    /// méridiens (clampés — minimum 3×2, maximum sous la limite u16 :
    /// jamais d'écrasement silencieux), normales RADIALES unitaires, UV
    /// équirectangulaires (φ/2π, θ/π), enroulement CCW vu de l'extérieur.
    pub fn sphere(center: [f32; 3], radius: f32, segments: u32, rings: u32) -> Self {
        let segments = segments.clamp(3, 254);
        let rings = rings.clamp(2, 254);
        let origin = Vec3::from_array(center);
        let mut vertices = Vec::with_capacity(((rings + 1) * (segments + 1)) as usize);
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * (ring as f32) / (rings as f32);
            let (sin_theta, cos_theta) = theta.sin_cos();
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * (segment as f32) / (segments as f32);
                let (sin_phi, cos_phi) = phi.sin_cos();
                let direction = Vec3::new(sin_theta * cos_phi, cos_theta, sin_theta * sin_phi);
                vertices.push(LitVertex {
                    position: (origin + direction * radius).to_array(),
                    normal: direction.to_array(),
                    uv: [segment as f32 / segments as f32, ring as f32 / rings as f32],
                });
            }
        }
        let mut indices = Vec::with_capacity((rings * segments * 6) as usize);
        let stride = segments + 1;
        for ring in 0..rings {
            for segment in 0..segments {
                let a = u16::try_from(ring * stride + segment).unwrap_or(u16::MAX);
                let b = u16::try_from((ring + 1) * stride + segment).unwrap_or(u16::MAX);
                indices.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
            }
        }
        Self { vertices, indices }
    }

    /// Quad éclairable dans le plan XY, normale +Z (cohérente avec
    /// l'enroulement CCW) : UV 0..`uv_scale`.
    pub fn quad(center: [f32; 3], width: f32, height: f32, uv_scale: f32) -> Self {
        let half_width = width / 2.0;
        let half_height = height / 2.0;
        let normal = [0.0, 0.0, 1.0];
        let vertices = vec![
            LitVertex {
                position: [center[0] - half_width, center[1] - half_height, center[2]],
                normal,
                uv: [0.0, uv_scale],
            },
            LitVertex {
                position: [center[0] + half_width, center[1] - half_height, center[2]],
                normal,
                uv: [uv_scale, uv_scale],
            },
            LitVertex {
                position: [center[0] + half_width, center[1] + half_height, center[2]],
                normal,
                uv: [uv_scale, 0.0],
            },
            LitVertex {
                position: [center[0] - half_width, center[1] + half_height, center[2]],
                normal,
                uv: [0.0, 0.0],
            },
        ];
        Self {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// La géométrie est-elle indexée ?
    pub fn is_indexed(&self) -> bool {
        !self.indices.is_empty()
    }

    /// Nombre d'éléments à dessiner : indices si indexé, sommets sinon.
    pub fn element_count(&self) -> u32 {
        let count = if self.is_indexed() {
            self.indices.len()
        } else {
            self.vertices.len()
        };
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Les sommets sérialisés pour un vertex buffer.
    pub fn vertex_bytes(&self) -> Vec<u8> {
        LitVertex::bytes_of(&self.vertices)
    }

    /// Les indices sérialisés pour un index buffer.
    pub fn index_bytes(&self) -> Vec<u8> {
        bytes_of_u16(&self.indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_is_not_indexed() {
        let colors = [Color::WHITE, Color::WHITE, Color::WHITE];
        let triangle = Geometry::triangle([0.0, 0.0, 0.0], 1.0, colors);
        assert_eq!(triangle.vertices.len(), 3);
        assert!(!triangle.is_indexed());
        assert_eq!(triangle.element_count(), 3);
        assert_eq!(
            triangle.vertex_bytes().len(),
            3 * ColorVertex::SIZE as usize
        );
    }

    #[test]
    fn quad_is_indexed_with_valid_indices() {
        let quad = Geometry::quad([0.0, 0.0, 0.0], 2.0, 1.0, Color::WHITE);
        assert_eq!(quad.vertices.len(), 4);
        assert!(quad.is_indexed());
        assert_eq!(quad.element_count(), 6);
        assert!(
            quad.indices
                .iter()
                .all(|index| usize::from(*index) < quad.vertices.len())
        );
        assert_eq!(quad.index_bytes().len(), 12);
    }

    #[test]
    fn center_offsets_every_vertex() {
        let quad = Geometry::quad([10.0, -5.0, 2.0], 2.0, 2.0, Color::WHITE);
        for vertex in &quad.vertices {
            assert!((vertex.position[0] - 10.0).abs() <= 1.0);
            assert!((vertex.position[1] + 5.0).abs() <= 1.0);
            assert_eq!(vertex.position[2], 2.0);
        }
    }

    #[test]
    fn cube_is_indexed_with_four_vertices_per_face() {
        let cube = Geometry::cube([0.0, 0.0, 0.0], 1.0, [Color::WHITE; 6]);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
        assert!(cube.is_indexed());
        assert_eq!(cube.element_count(), 36);
        assert!(
            cube.indices
                .iter()
                .all(|index| usize::from(*index) < cube.vertices.len())
        );
    }

    #[test]
    fn cube_colors_are_uniform_per_face() {
        let face_colors = [
            Color::rgb(1.0, 0.0, 0.0),
            Color::rgb(0.0, 1.0, 0.0),
            Color::rgb(0.0, 0.0, 1.0),
            Color::rgb(1.0, 1.0, 0.0),
            Color::rgb(0.0, 1.0, 1.0),
            Color::rgb(1.0, 0.0, 1.0),
        ];
        let cube = Geometry::cube([0.0, 0.0, 0.0], 2.0, face_colors);
        for (face, color) in face_colors.iter().enumerate() {
            for vertex in &cube.vertices[face * 4..face * 4 + 4] {
                assert_eq!(vertex.color, [color.r, color.g, color.b]);
            }
        }
    }

    #[test]
    fn cube_winding_is_ccw_seen_from_outside() {
        let center = Vec3::new(1.0, -2.0, 3.0);
        let cube = Geometry::cube(center.to_array(), 2.0, [Color::WHITE; 6]);
        for triangle in cube.indices.chunks(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                .map(|index| Vec3::from_array(cube.vertices[usize::from(index)].position));
            let normal = (b - a).cross(c - a);
            let centroid = (a + b + c) / 3.0;
            assert!(normal.dot(centroid - center) > 0.0);
        }
    }

    #[test]
    fn cube_center_offsets_every_vertex() {
        let cube = Geometry::cube([10.0, -5.0, 2.0], 2.0, [Color::WHITE; 6]);
        for vertex in &cube.vertices {
            assert!((vertex.position[0] - 10.0).abs() <= 1.0);
            assert!((vertex.position[1] + 5.0).abs() <= 1.0);
            assert!((vertex.position[2] - 2.0).abs() <= 1.0);
        }
    }

    #[test]
    fn textured_quad_is_indexed_with_top_left_uv_origin() {
        let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 2.0, 2.0, 1.0);
        assert_eq!(quad.vertices.len(), 4);
        assert!(quad.is_indexed());
        assert_eq!(quad.element_count(), 6);
        let top_left = quad
            .vertices
            .iter()
            .find(|vertex| vertex.position[0] < 0.0 && vertex.position[1] > 0.0)
            .unwrap();
        assert_eq!(top_left.uv, [0.0, 0.0]);
        let bottom_right = quad
            .vertices
            .iter()
            .find(|vertex| vertex.position[0] > 0.0 && vertex.position[1] < 0.0)
            .unwrap();
        assert_eq!(bottom_right.uv, [1.0, 1.0]);
    }

    #[test]
    fn textured_quad_uv_scale_stretches_the_coordinates() {
        let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 4.0);
        assert!(quad.vertices.iter().any(|vertex| vertex.uv == [4.0, 4.0]));
        assert_eq!(quad.vertex_bytes().len(), 4 * 20);
        assert_eq!(quad.index_bytes().len(), 12);
    }

    #[test]
    fn textured_cube_maps_full_uv_range_on_every_face() {
        let cube = TexturedGeometry::cube([0.0, 0.0, 0.0], 1.0);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
        assert_eq!(cube.element_count(), 36);
        for face in cube.vertices.chunks(4) {
            let mut uvs: Vec<[f32; 2]> = face.iter().map(|vertex| vertex.uv).collect();
            uvs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(
                uvs,
                vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
                "chaque face doit couvrir tout le carré UV"
            );
        }
    }

    #[test]
    fn textured_cube_winding_is_ccw_seen_from_outside() {
        let center = Vec3::new(-1.0, 2.0, 0.5);
        let cube = TexturedGeometry::cube(center.to_array(), 2.0);
        for triangle in cube.indices.chunks(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                .map(|index| Vec3::from_array(cube.vertices[usize::from(index)].position));
            let normal = (b - a).cross(c - a);
            let centroid = (a + b + c) / 3.0;
            assert!(normal.dot(centroid - center) > 0.0);
        }
    }

    #[test]
    fn the_sphere_is_radial_and_ccw() {
        let center = Vec3::new(1.0, -2.0, 0.5);
        let sphere = LitGeometry::sphere(center.to_array(), 2.0, 12, 8);
        assert_eq!(sphere.vertices.len(), 13 * 9);
        assert_eq!(sphere.indices.len(), (12 * 8 * 6) as usize);
        for vertex in &sphere.vertices {
            let normal = Vec3::from_array(vertex.normal);
            assert!((normal.length() - 1.0).abs() < 1e-5);
            let outward = (Vec3::from_array(vertex.position) - center).normalize();
            assert!(normal.dot(outward) > 0.999);
            assert!((0.0..=1.0).contains(&vertex.uv[0]));
            assert!((0.0..=1.0).contains(&vertex.uv[1]));
        }
        // Enroulement CCW vu de l'extérieur (les triangles dégénérés des
        // pôles, d'aire nulle, sont tolérés).
        for triangle in sphere.indices.chunks(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                .map(|index| Vec3::from_array(sphere.vertices[usize::from(index)].position));
            let normal = (b - a).cross(c - a);
            if normal.length_squared() > 1e-9 {
                let centroid = (a + b + c) / 3.0;
                assert!(normal.dot(centroid - center) > 0.0);
            }
        }
    }

    #[test]
    fn the_sphere_clamps_its_resolution() {
        // Jamais d'écrasement u16 silencieux : la résolution est bornée
        // à la construction, en bas comme en haut.
        let tiny = LitGeometry::sphere([0.0; 3], 1.0, 0, 0);
        assert_eq!(tiny.vertices.len(), 4 * 3);
        let huge = LitGeometry::sphere([0.0; 3], 1.0, 10_000, 10_000);
        assert_eq!(huge.vertices.len(), 255 * 255);
        assert!(huge.vertices.len() <= usize::from(u16::MAX));
    }
}
