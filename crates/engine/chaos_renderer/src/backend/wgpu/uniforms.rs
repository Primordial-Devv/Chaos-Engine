use log::debug;

const MATRIX_SIZE: u64 = 64;

/// Mécanique des uniforms du backend — convention de binding du moteur :
/// group(0) = données de frame (view-projection), group(1) = données d'objet
/// (matrice modèle), un slot par draw, réutilisés à chaque frame.
/// Les dynamic offsets sont l'optimisation prévue pour le render queue.
pub(super) struct Uniforms {
    pub(super) frame_layout: wgpu::BindGroupLayout,
    pub(super) object_layout: wgpu::BindGroupLayout,
    frame_buffer: wgpu::Buffer,
    pub(super) frame_bind_group: wgpu::BindGroup,
    object_slots: Vec<ObjectSlot>,
}

struct ObjectSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Uniforms {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let frame_layout = uniform_layout(device, "chaos.frame_uniforms");
        let object_layout = uniform_layout(device, "chaos.object_uniforms");
        let frame_buffer = uniform_buffer(device, "chaos.frame_uniforms");
        let frame_bind_group =
            uniform_bind_group(device, "chaos.frame_uniforms", &frame_layout, &frame_buffer);
        Self {
            frame_layout,
            object_layout,
            frame_buffer,
            frame_bind_group,
            object_slots: Vec::new(),
        }
    }

    pub(super) fn write_frame(&self, queue: &wgpu::Queue, bytes: &[u8; 64]) {
        queue.write_buffer(&self.frame_buffer, 0, bytes);
    }

    pub(super) fn ensure_object_slots(&mut self, device: &wgpu::Device, count: usize) {
        while self.object_slots.len() < count {
            let index = self.object_slots.len();
            let label = format!("chaos.object_uniforms.{index}");
            let buffer = uniform_buffer(device, &label);
            let bind_group = uniform_bind_group(device, &label, &self.object_layout, &buffer);
            self.object_slots.push(ObjectSlot { buffer, bind_group });
            debug!("object uniform slots grown to {}", self.object_slots.len());
        }
    }

    pub(super) fn write_object(&self, queue: &wgpu::Queue, index: usize, bytes: &[u8; 64]) {
        if let Some(slot) = self.object_slots.get(index) {
            queue.write_buffer(&slot.buffer, 0, bytes);
        }
    }

    pub(super) fn object_bind_group(&self, index: usize) -> Option<&wgpu::BindGroup> {
        self.object_slots.get(index).map(|slot| &slot.bind_group)
    }
}

fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(MATRIX_SIZE),
            },
            count: None,
        }],
    })
}

fn uniform_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: MATRIX_SIZE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn uniform_bind_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}
