use chaos_core::Color;

use crate::resources::{ColorVertex, bytes_of_u16};

/// Géométrie côté CPU : la donnée moteur, indépendante de toute
/// représentation GPU. `indices` vide = rendu non indexé.
/// Indices en u16 pour l'instant ; u32 viendra avec les gros meshes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    pub vertices: Vec<ColorVertex>,
    pub indices: Vec<u16>,
}

impl Geometry {
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

    pub fn vertex_bytes(&self) -> Vec<u8> {
        ColorVertex::bytes_of(&self.vertices)
    }

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
}
