use log::debug;

use crate::resources::DebugVertex;

/// Le stride d'UN sommet de debug dans le buffer — le miroir exact de
/// `DebugVertex::layout()` (position + couleur RGBA), verrouillé par
/// test naga côté shader.
pub(super) const DEBUG_VERTEX_STRIDE: u64 = DebugVertex::SIZE as u64;

/// Le BUFFER DE DEBUG du backend : les sommets lignes des batches de
/// debug, dans un buffer VERTEX croissant — le patron de l'instance
/// buffer. PARTAGÉ entre les passes : chaque passe écrit SES sommets
/// puis SOUMET (le contrat write → submit par passe) — la timeline de
/// queue garantit l'ordre. La croissance recrée le buffer (l'ancien
/// survit aux soumissions en vol, garantie wgpu).
pub(super) struct DebugBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl DebugBuffer {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let capacity = 256 * DEBUG_VERTEX_STRIDE;
        Self {
            buffer: create_buffer(device, capacity),
            capacity,
        }
    }

    /// Écrit les octets de sommets de LA passe qui va être soumise — le
    /// buffer croît à la demande (au moins doublé, jamais rétréci).
    pub(super) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let needed = bytes.len() as u64;
        if needed > self.capacity {
            self.capacity = needed.max(self.capacity * 2);
            self.buffer = create_buffer(device, self.capacity);
            debug!("debug vertex buffer grown to {} bytes", self.capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }

    /// La tranche du buffer à partir du PREMIER sommet d'un batch —
    /// bindée au slot 0, la plage de sommets démarre à 0.
    pub(super) fn slice_from(&self, first_vertex: u32) -> wgpu::BufferSlice<'_> {
        self.buffer
            .slice(u64::from(first_vertex) * DEBUG_VERTEX_STRIDE..)
    }
}

fn create_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chaos.debug_vertices"),
        size: capacity,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
