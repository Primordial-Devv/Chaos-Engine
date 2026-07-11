use chaos_core::math::Aabb;

use crate::resources::{BufferHandle, VertexLayout};

/// Identifiant opaque d'un mesh. Générationnel : un handle dont le mesh a
/// été détruit est détecté, jamais résolu vers un autre mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Ressource mesh côté renderer : géométrie résidente GPU prête à dessiner —
/// buffers possédés, draw info, vertex format et BOUNDS locaux (l'AABB
/// des positions, calculé à la création — `None` = jamais cullé, le
/// défaut sûr des géométries vides ou dégénérées).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeshRecord {
    pub(crate) vertex_buffer: BufferHandle,
    pub(crate) index_buffer: Option<BufferHandle>,
    pub(crate) element_count: u32,
    pub(crate) vertex_layout: VertexLayout,
    pub(crate) bounds: Option<Aabb>,
}
