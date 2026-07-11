use log::debug;

/// Le stride d'UNE instance dans le buffer — le miroir exact du layout
/// d'instance (`resources::instance_transforms_layout()`, verrouillé par
/// test) : matrice modèle (64 o) + matrice des normales (64 o).
pub(super) const INSTANCE_STRIDE: u64 = 128;

/// L'INSTANCE BUFFER du backend : les transforms par instance des draws
/// instanciés, dans un buffer VERTEX croissant — le pendant « par
/// instance » des slots d'uniforms d'objets. PARTAGÉ entre les passes :
/// chaque passe écrit SES instances puis SOUMET (le contrat
/// write → submit par passe, la passe d'ombre comprise) — la timeline
/// de queue garantit l'ordre. La croissance recrée le buffer (l'ancien
/// survit aux soumissions en vol, garantie wgpu).
pub(super) struct InstanceBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl InstanceBuffer {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let capacity = 64 * INSTANCE_STRIDE;
        Self {
            buffer: create_buffer(device, capacity),
            capacity,
        }
    }

    /// Écrit les octets d'instances de LA passe qui va être soumise —
    /// le buffer croît à la demande (au moins doublé, jamais rétréci).
    pub(super) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let needed = bytes.len() as u64;
        if needed > self.capacity {
            self.capacity = needed.max(self.capacity * 2);
            self.buffer = create_buffer(device, self.capacity);
            debug!("instance buffer grown to {} bytes", self.capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }

    /// La tranche du buffer à partir de la PREMIÈRE instance d'un draw —
    /// bindée au slot 1, la plage d'instances démarre à 0 (jamais de
    /// `first_instance` non nul).
    pub(super) fn slice_from(&self, first_instance: u32) -> wgpu::BufferSlice<'_> {
        self.buffer
            .slice(u64::from(first_instance) * INSTANCE_STRIDE..)
    }
}

fn create_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chaos.instances"),
        size: capacity,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
