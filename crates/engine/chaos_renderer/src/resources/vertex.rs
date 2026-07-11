/// Format d'un attribut de vertex. Extensible au besoin (formats entiers
/// et normalisés pour le skinning et les couleurs packées, notamment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexAttributeFormat {
    /// Deux `f32` — UV, positions 2D.
    Float32x2,
    /// Trois `f32` — positions 3D, couleurs RGB, normales.
    Float32x3,
    /// Quatre `f32` — couleurs RGBA, tangentes.
    Float32x4,
}

impl VertexAttributeFormat {
    /// La taille de l'attribut en octets.
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
    /// La `@location` WGSL correspondante.
    pub location: u32,
    /// Le format de la donnée.
    pub format: VertexAttributeFormat,
    /// L'offset en octets dans le vertex.
    pub offset: u32,
}

/// Cadence de lecture du buffer : par sommet, ou par instance (instancing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VertexStepMode {
    /// Le buffer avance à chaque SOMMET — le défaut.
    #[default]
    Vertex,
    /// Le buffer avance à chaque INSTANCE (instancing).
    Instance,
}

/// Layout déclaratif d'un vertex buffer, défini côté Chaos — le backend le
/// convertit vers sa représentation native. Un seul slot de layout par
/// pipeline pour l'instant ; le multi-slots viendra avec l'instancing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    /// La taille d'un vertex complet en octets.
    pub stride: u32,
    /// La cadence de lecture du buffer.
    pub step_mode: VertexStepMode,
    /// Les attributs, dans l'ordre des `@location`.
    pub attributes: Vec<VertexAttribute>,
}

impl VertexLayout {
    /// Constructeur déclaratif pour des attributs entrelacés et contigus :
    /// locations 0..n, offsets cumulés, stride calculé.
    pub fn packed(formats: &[VertexAttributeFormat]) -> Self {
        Self::packed_at(0, VertexStepMode::Vertex, formats)
    }

    /// Le constructeur GÉNÉRALISÉ de [`VertexLayout::packed`] : les
    /// locations démarrent à `base_location` et la cadence est choisie —
    /// le constructeur des layouts d'INSTANCE, dont les attributs vivent
    /// au-dessus de ceux du mesh.
    pub fn packed_at(
        base_location: u32,
        step_mode: VertexStepMode,
        formats: &[VertexAttributeFormat],
    ) -> Self {
        let mut attributes = Vec::with_capacity(formats.len());
        let mut offset = 0;
        for (index, format) in formats.iter().enumerate() {
            attributes.push(VertexAttribute {
                location: base_location.saturating_add(u32::try_from(index).unwrap_or(u32::MAX)),
                format: *format,
                offset,
            });
            offset += format.size();
        }
        Self {
            stride: offset,
            step_mode,
            attributes,
        }
    }
}

/// Le layout d'INSTANCE du moteur — l'AUTORITÉ unique, le miroir des
/// `ObjectUniforms` en cadence Instance : la matrice modèle (locations
/// 4..=7) puis la matrice des normales (locations 8..=11), huit
/// `Float32x4` entrelacés, stride 128 octets. Les locations démarrent à
/// [`crate::shaders::inputs::INSTANCE_LOCATION_BASE`] — au-dessus des
/// attributs de tous les layouts de mesh builtin. Consommé par les
/// entrées `vs_instanced` des shaders intégrés.
pub fn instance_transforms_layout() -> VertexLayout {
    VertexLayout::packed_at(
        crate::shaders::inputs::INSTANCE_LOCATION_BASE,
        VertexStepMode::Instance,
        &[VertexAttributeFormat::Float32x4; 8],
    )
}

/// Le vertex standard du moteur : position 3D + couleur. Il se décrit
/// lui-même via le système de layouts — plus un cas spécial du backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorVertex {
    /// La position dans l'espace objet.
    pub position: [f32; 3],
    /// La couleur RGB linéaire du sommet.
    pub color: [f32; 3],
}

impl ColorVertex {
    /// La taille du vertex en octets (position + couleur).
    pub const SIZE: u32 = 24;

    /// Le layout déclaratif correspondant (`@location(0)` position,
    /// `@location(1)` couleur).
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x3,
        ])
    }

    /// Sérialise les sommets en octets natifs entrelacés, prêts pour un
    /// vertex buffer.
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

/// Le vertex du DEBUG RENDERING : position monde + couleur RGBA (l'alpha
/// voyage — les pipelines debug mélangent). Consommé par le shader
/// `chaos.debug` en topologie lignes ; les sommets sont PRÉ-TRANSFORMÉS
/// en espace monde (aucune matrice modèle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugVertex {
    /// La position en espace MONDE.
    pub position: [f32; 3],
    /// La couleur RGBA linéaire du sommet.
    pub color: [f32; 4],
}

impl DebugVertex {
    /// La taille du vertex en octets (position + couleur RGBA).
    pub const SIZE: u32 = 28;

    /// Le layout déclaratif correspondant (`@location(0)` position,
    /// `@location(1)` couleur RGBA) — l'AUTORITÉ du miroir WGSL,
    /// verrouillée par test naga.
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x4,
        ])
    }

    /// Sérialise les sommets en octets natifs entrelacés, prêts pour un
    /// vertex buffer.
    pub fn bytes_of(vertices: &[DebugVertex]) -> Vec<u8> {
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
    /// La position dans l'espace objet.
    pub position: [f32; 3],
    /// Les coordonnées de texture (origine en haut à gauche).
    pub uv: [f32; 2],
}

impl TexturedVertex {
    /// La taille du vertex en octets (position + UV).
    pub const SIZE: u32 = 20;

    /// Le layout déclaratif correspondant (`@location(0)` position,
    /// `@location(1)` UV).
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x2,
        ])
    }

    /// Sérialise les sommets en octets natifs entrelacés, prêts pour un
    /// vertex buffer.
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

/// Le troisième vertex standard, celui des surfaces ÉCLAIRÉES : position
/// 3D + normale (espace objet, unitaire) + coordonnées de texture. Les
/// tangentes (normal mapping) l'étendront avec leur besoin réel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LitVertex {
    /// La position dans l'espace objet.
    pub position: [f32; 3],
    /// La normale de surface dans l'espace objet (unitaire).
    pub normal: [f32; 3],
    /// Les coordonnées de texture (origine en haut à gauche).
    pub uv: [f32; 2],
}

impl LitVertex {
    /// La taille du vertex en octets (position + normale + UV).
    pub const SIZE: u32 = 32;

    /// Le layout déclaratif correspondant (`@location(0)` position,
    /// `@location(1)` normale, `@location(2)` UV).
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x3,
            VertexAttributeFormat::Float32x2,
        ])
    }

    /// Sérialise les sommets en octets natifs entrelacés, prêts pour un
    /// vertex buffer.
    pub fn bytes_of(vertices: &[LitVertex]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(vertices.len() * Self::SIZE as usize);
        for vertex in vertices {
            for value in vertex
                .position
                .iter()
                .chain(vertex.normal.iter())
                .chain(vertex.uv.iter())
            {
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
    fn debug_vertex_layout_matches_its_size() {
        let layout = DebugVertex::layout();
        assert_eq!(layout.stride, DebugVertex::SIZE);
        assert_eq!(layout.attributes.len(), 2);
        assert_eq!(layout.attributes[1].offset, 12);
        assert_eq!(
            layout.attributes[1].format,
            VertexAttributeFormat::Float32x4
        );
        let bytes = DebugVertex::bytes_of(&[DebugVertex {
            position: [1.0, 2.0, 3.0],
            color: [0.5, 0.25, 0.125, 0.75],
        }]);
        assert_eq!(bytes.len(), DebugVertex::SIZE as usize);
        assert_eq!(bytes[12..16], 0.5f32.to_ne_bytes());
        assert_eq!(bytes[24..28], 0.75f32.to_ne_bytes());
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

    #[test]
    fn packed_at_offsets_the_locations_from_the_base() {
        let layout = VertexLayout::packed_at(
            3,
            VertexStepMode::Instance,
            &[
                VertexAttributeFormat::Float32x2,
                VertexAttributeFormat::Float32x3,
            ],
        );
        assert_eq!(layout.attributes[0].location, 3);
        assert_eq!(layout.attributes[0].offset, 0);
        assert_eq!(layout.attributes[1].location, 4);
        assert_eq!(layout.attributes[1].offset, 8);
        assert_eq!(layout.stride, 20);
        assert_eq!(layout.step_mode, VertexStepMode::Instance);
    }

    #[test]
    fn the_instance_layout_mirrors_the_object_uniforms() {
        // L'autorité du layout d'instance : 128 octets (modèle + normale),
        // huit Float32x4 aux locations 4..=11, cadence Instance — le
        // miroir des ObjectUniforms consommé par les vs_instanced.
        let layout = instance_transforms_layout();
        assert_eq!(layout.stride, 128);
        assert_eq!(layout.step_mode, VertexStepMode::Instance);
        assert_eq!(layout.attributes.len(), 8);
        for (index, attribute) in layout.attributes.iter().enumerate() {
            let index = u32::try_from(index).unwrap();
            assert_eq!(attribute.location, 4 + index);
            assert_eq!(attribute.format, VertexAttributeFormat::Float32x4);
            assert_eq!(attribute.offset, index * 16);
        }
    }
}
