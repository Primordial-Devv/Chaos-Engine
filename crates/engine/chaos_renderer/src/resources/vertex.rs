/// Format d'un attribut de vertex. Extensible au besoin (formats entiers
/// et normalisés pour le skinning et les couleurs packées, notamment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexAttributeFormat {
    Float32x2,
    Float32x3,
    Float32x4,
}

impl VertexAttributeFormat {
    pub fn size(self) -> u32 {
        match self {
            Self::Float32x2 => 8,
            Self::Float32x3 => 12,
            Self::Float32x4 => 16,
        }
    }
}

/// Un attribut du layout : sa `location` WGSL, son format, son offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    pub location: u32,
    pub format: VertexAttributeFormat,
    pub offset: u32,
}

/// Cadence de lecture du buffer : par sommet, ou par instance (instancing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VertexStepMode {
    #[default]
    Vertex,
    Instance,
}

/// Layout déclaratif d'un vertex buffer, défini côté Chaos — le backend le
/// convertit vers sa représentation native. Un seul slot de layout par
/// pipeline pour l'instant ; le multi-slots viendra avec l'instancing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexLayout {
    pub stride: u32,
    pub step_mode: VertexStepMode,
    pub attributes: Vec<VertexAttribute>,
}

impl VertexLayout {
    /// Constructeur déclaratif pour des attributs entrelacés et contigus :
    /// locations 0..n, offsets cumulés, stride calculé.
    pub fn packed(formats: &[VertexAttributeFormat]) -> Self {
        let mut attributes = Vec::with_capacity(formats.len());
        let mut offset = 0;
        for (index, format) in formats.iter().enumerate() {
            attributes.push(VertexAttribute {
                location: u32::try_from(index).unwrap_or(u32::MAX),
                format: *format,
                offset,
            });
            offset += format.size();
        }
        Self {
            stride: offset,
            step_mode: VertexStepMode::Vertex,
            attributes,
        }
    }
}

/// Le vertex standard du moteur : position 3D + couleur. Il se décrit
/// lui-même via le système de layouts — plus un cas spécial du backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl ColorVertex {
    pub const SIZE: u32 = 24;

    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x3,
        ])
    }

    pub fn bytes_of(vertices: &[ColorVertex]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(vertices.len() * Self::SIZE as usize);
        for vertex in vertices {
            for value in vertex.position.iter().chain(vertex.color.iter()) {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        bytes
    }
}

/// Le deuxième vertex standard : position 3D + coordonnées de texture
/// (origine en haut à gauche — convention verrouillée dans
/// `docs/architecture/math-conventions.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexturedVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

impl TexturedVertex {
    pub const SIZE: u32 = 20;

    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x2,
        ])
    }

    pub fn bytes_of(vertices: &[TexturedVertex]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(vertices.len() * Self::SIZE as usize);
        for vertex in vertices {
            for value in vertex.position.iter().chain(vertex.uv.iter()) {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_computes_offsets_and_stride() {
        let layout = VertexLayout::packed(&[
            VertexAttributeFormat::Float32x2,
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x4,
        ]);
        assert_eq!(layout.stride, 36);
        assert_eq!(layout.step_mode, VertexStepMode::Vertex);
        let described: Vec<(u32, u32)> = layout
            .attributes
            .iter()
            .map(|attribute| (attribute.location, attribute.offset))
            .collect();
        assert_eq!(described, vec![(0, 0), (1, 8), (2, 20)]);
    }

    #[test]
    fn color_vertex_layout_matches_its_size() {
        let layout = ColorVertex::layout();
        assert_eq!(layout.stride, ColorVertex::SIZE);
        assert_eq!(layout.attributes.len(), 2);
        assert_eq!(layout.attributes[1].offset, 12);
    }

    #[test]
    fn textured_vertex_layout_matches_its_size() {
        let layout = TexturedVertex::layout();
        assert_eq!(layout.stride, TexturedVertex::SIZE);
        assert_eq!(layout.attributes.len(), 2);
        assert_eq!(layout.attributes[1].offset, 12);
        assert_eq!(
            layout.attributes[1].format,
            VertexAttributeFormat::Float32x2
        );
    }

    #[test]
    fn textured_bytes_of_produces_interleaved_native_bytes() {
        let vertices = [TexturedVertex {
            position: [1.0, 2.0, 3.0],
            uv: [0.25, 0.75],
        }];
        let bytes = TexturedVertex::bytes_of(&vertices);
        assert_eq!(bytes.len(), TexturedVertex::SIZE as usize);
        assert_eq!(bytes[..4], 1.0f32.to_ne_bytes());
        assert_eq!(bytes[12..16], 0.25f32.to_ne_bytes());
        assert_eq!(bytes[16..20], 0.75f32.to_ne_bytes());
    }

    #[test]
    fn bytes_of_produces_interleaved_native_bytes() {
        let vertices = [
            ColorVertex {
                position: [1.0, 2.0, 3.0],
                color: [0.5, 0.25, 0.125],
            },
            ColorVertex {
                position: [-1.0, -2.0, -3.0],
                color: [1.0, 1.0, 1.0],
            },
        ];
        let bytes = ColorVertex::bytes_of(&vertices);
        assert_eq!(bytes.len(), 2 * ColorVertex::SIZE as usize);
        assert_eq!(bytes[..4], 1.0f32.to_ne_bytes());
        assert_eq!(bytes[12..16], 0.5f32.to_ne_bytes());
        assert_eq!(bytes[24..28], (-1.0f32).to_ne_bytes());
    }
}
