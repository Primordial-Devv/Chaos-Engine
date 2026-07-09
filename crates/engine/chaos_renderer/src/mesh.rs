use crate::resources::{BufferHandle, VertexLayout};

/// Identifiant opaque d'un mesh. Générationnel : un handle dont le mesh a
/// été détruit est détecté, jamais résolu vers un autre mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Ressource mesh côté renderer : géométrie résidente GPU prête à dessiner —
/// buffers possédés, draw info et vertex format. Portera les bounds (AABB)
/// quand le culling en aura besoin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeshRecord {
    pub(crate) vertex_buffer: BufferHandle,
    pub(crate) index_buffer: Option<BufferHandle>,
    pub(crate) element_count: u32,
    pub(crate) vertex_layout: VertexLayout,
}
