use log::debug;

use crate::shaders::inputs;

use super::depth;

/// Les uniforms de frame : matrice vue-projection + position caméra
/// (vec4 — le spéculaire PBR) + matrice vue-projection INVERSE (la
/// déprojection du ciel) + paramètres d'environnement (intensité,
/// exposition) — le miroir exact de `FrameUniforms` dans
/// `shaders/pbr.wgsl` et `shaders/sky.wgsl`, packé par
/// `convert::frame_to_bytes`.
pub(super) const FRAME_UNIFORMS_SIZE: usize = 160;
/// Les uniforms d'objet : matrice modèle + matrice des normales.
pub(super) const OBJECT_UNIFORMS_SIZE: usize = 128;
/// Le buffer des lumières : ambiante (16 o) + compte (16 o) + 16 lumières
/// de 64 octets + la queue OMBRE (vue-projection de lumière @1056,
/// paramètres @1120) — le miroir exact de `LightsUniforms` dans
/// `shaders/lit.wgsl`, packé par `convert::lights_to_bytes`.
pub(super) const LIGHTS_UNIFORMS_SIZE: usize = 1136;

/// Mécanique des uniforms du backend — convention de binding du moteur :
/// group(0) = données de FRAME (view-projection au binding 0, lumières au
/// binding 1, cubemap d'environnement au binding 2, son sampler au
/// binding 3, shadow map au binding 4, son sampler de COMPARAISON au
/// binding 5 — un seul bind group pour tous), group(1) = données
/// d'objet (modèle + normales), un slot par draw, réutilisés à chaque
/// frame. Les shaders qui ne déclarent pas tous les bindings du groupe
/// restent valides sous ce layout (la règle WebGPU). Les vues VIVES
/// (environnement, ombre) sont RETENUES : chaque rebind reconstruit le
/// bind group depuis l'état complet — rebinder l'une ne perd jamais
/// l'autre. Les dynamic offsets sont l'optimisation prévue pour le
/// render queue.
pub(super) struct Uniforms {
    pub(super) frame_layout: wgpu::BindGroupLayout,
    pub(super) object_layout: wgpu::BindGroupLayout,
    frame_buffer: wgpu::Buffer,
    lights_buffer: wgpu::Buffer,
    environment_fallback_view: wgpu::TextureView,
    environment_sampler: wgpu::Sampler,
    environment_view: Option<wgpu::TextureView>,
    shadow_fallback_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_view: Option<wgpu::TextureView>,
    pub(super) frame_bind_group: wgpu::BindGroup,
    /// Le groupe(0) RÉDUIT de la passe d'ombre (le buffer frame seul) :
    /// binder le groupe frame complet pendant cette passe serait un
    /// conflit d'usage wgpu — la shadow map y est bindée en texture
    /// alors qu'elle est l'attachement de profondeur de la passe.
    pub(super) shadow_frame_layout: wgpu::BindGroupLayout,
    pub(super) shadow_frame_bind_group: wgpu::BindGroup,
    object_slots: Vec<ObjectSlot>,
}

struct ObjectSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Uniforms {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chaos.frame_uniforms"),
            entries: &[
                uniform_layout_entry(inputs::FRAME_UNIFORMS_BINDING, FRAME_UNIFORMS_SIZE as u64),
                uniform_layout_entry(inputs::FRAME_LIGHTS_BINDING, LIGHTS_UNIFORMS_SIZE as u64),
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::FRAME_ENVIRONMENT_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::FRAME_ENVIRONMENT_SAMPLER_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::FRAME_SHADOW_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::FRAME_SHADOW_SAMPLER_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chaos.object_uniforms"),
            entries: &[uniform_layout_entry(
                inputs::OBJECT_UNIFORMS_BINDING,
                OBJECT_UNIFORMS_SIZE as u64,
            )],
        });
        let frame_buffer =
            uniform_buffer(device, "chaos.frame_uniforms", FRAME_UNIFORMS_SIZE as u64);
        let lights_buffer =
            uniform_buffer(device, "chaos.frame_lights", LIGHTS_UNIFORMS_SIZE as u64);
        // Le cube fallback : 1×1×6, zéro-initialisé par wgpu — noir sans
        // upload. Organe interne du backend, hors du pool (comme la
        // profondeur) ; la vue le garde vivant.
        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chaos.environment_fallback"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let environment_fallback_view =
            fallback_texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("chaos.environment_fallback"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
        let environment_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("chaos.environment_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // La profondeur fallback des ombres : 1×1 effacée à 1.0 — la
        // comparaison réussit partout, « tout éclairé » sans branche
        // shader obligatoire (le patron du cube d'environnement noir).
        let shadow_fallback_view =
            depth::create_sampleable_depth_view(device, 1, "chaos.shadow_fallback");
        depth::clear_depth_to_one(
            device,
            queue,
            &shadow_fallback_view,
            "chaos.shadow_fallback",
        );
        // Le sampler de COMPARAISON des ombres : LessEqual (plus proche
        // ou égal = éclairé), Linear = le PCF matériel 2×2 par tap.
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("chaos.shadow_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let frame_bind_group = frame_bind_group(
            device,
            &frame_layout,
            &frame_buffer,
            &lights_buffer,
            &environment_fallback_view,
            &environment_sampler,
            &shadow_fallback_view,
            &shadow_sampler,
        );
        let shadow_frame_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("chaos.shadow_frame_uniforms"),
                entries: &[uniform_layout_entry(
                    inputs::FRAME_UNIFORMS_BINDING,
                    FRAME_UNIFORMS_SIZE as u64,
                )],
            });
        let shadow_frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chaos.shadow_frame_uniforms"),
            layout: &shadow_frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: inputs::FRAME_UNIFORMS_BINDING,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        Self {
            frame_layout,
            object_layout,
            frame_buffer,
            lights_buffer,
            environment_fallback_view,
            environment_sampler,
            environment_view: None,
            shadow_fallback_view,
            shadow_sampler,
            shadow_view: None,
            frame_bind_group,
            shadow_frame_layout,
            shadow_frame_bind_group,
            object_slots: Vec::new(),
        }
    }

    /// Rebinde la cubemap d'environnement du groupe frame — `None`
    /// rebinde le cube fallback noir. La vue est RETENUE et le bind
    /// group reconstruit depuis l'état complet (l'ombre vive survit) ;
    /// les soumissions en vol référencent l'ancien, wgpu garantit sa
    /// survie.
    pub(super) fn rebind_environment(
        &mut self,
        device: &wgpu::Device,
        view: Option<wgpu::TextureView>,
    ) {
        self.environment_view = view;
        self.rebuild_frame_bind_group(device);
    }

    /// Rebinde la shadow map du groupe frame — `None` rebinde le
    /// fallback « tout éclairé ». La vue est RETENUE et le bind group
    /// reconstruit depuis l'état complet (l'environnement vif survit).
    pub(super) fn rebind_shadow(&mut self, device: &wgpu::Device, view: Option<wgpu::TextureView>) {
        self.shadow_view = view;
        self.rebuild_frame_bind_group(device);
    }

    fn rebuild_frame_bind_group(&mut self, device: &wgpu::Device) {
        self.frame_bind_group = frame_bind_group(
            device,
            &self.frame_layout,
            &self.frame_buffer,
            &self.lights_buffer,
            self.environment_view
                .as_ref()
                .unwrap_or(&self.environment_fallback_view),
            &self.environment_sampler,
            self.shadow_view
                .as_ref()
                .unwrap_or(&self.shadow_fallback_view),
            &self.shadow_sampler,
        );
    }

    pub(super) fn write_frame(&self, queue: &wgpu::Queue, bytes: &[u8; FRAME_UNIFORMS_SIZE]) {
        queue.write_buffer(&self.frame_buffer, 0, bytes);
    }

    /// Écrit l'éclairage de la frame — UNE fois par plan (les lumières
    /// sont constantes sur toutes les passes du plan : une écriture
    /// stagée avant le premier submit s'applique à tous les suivants).
    pub(super) fn write_lights(&self, queue: &wgpu::Queue, bytes: &[u8; LIGHTS_UNIFORMS_SIZE]) {
        queue.write_buffer(&self.lights_buffer, 0, bytes);
    }

    pub(super) fn ensure_object_slots(&mut self, device: &wgpu::Device, count: usize) {
        while self.object_slots.len() < count {
            let index = self.object_slots.len();
            let label = format!("chaos.object_uniforms.{index}");
            let buffer = uniform_buffer(device, &label, OBJECT_UNIFORMS_SIZE as u64);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&label),
                layout: &self.object_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: inputs::OBJECT_UNIFORMS_BINDING,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.object_slots.push(ObjectSlot { buffer, bind_group });
            debug!("object uniform slots grown to {}", self.object_slots.len());
        }
    }

    pub(super) fn write_object(
        &self,
        queue: &wgpu::Queue,
        index: usize,
        bytes: &[u8; OBJECT_UNIFORMS_SIZE],
    ) {
        if let Some(slot) = self.object_slots.get(index) {
            queue.write_buffer(&slot.buffer, 0, bytes);
        }
    }

    pub(super) fn object_bind_group(&self, index: usize) -> Option<&wgpu::BindGroup> {
        self.object_slots.get(index).map(|slot| &slot.bind_group)
    }
}

#[allow(clippy::too_many_arguments)]
fn frame_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame_buffer: &wgpu::Buffer,
    lights_buffer: &wgpu::Buffer,
    environment_view: &wgpu::TextureView,
    environment_sampler: &wgpu::Sampler,
    shadow_view: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chaos.frame_uniforms"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_UNIFORMS_BINDING,
                resource: frame_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_LIGHTS_BINDING,
                resource: lights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_ENVIRONMENT_BINDING,
                resource: wgpu::BindingResource::TextureView(environment_view),
            },
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_ENVIRONMENT_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(environment_sampler),
            },
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_SHADOW_BINDING,
                resource: wgpu::BindingResource::TextureView(shadow_view),
            },
            wgpu::BindGroupEntry {
                binding: inputs::FRAME_SHADOW_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(shadow_sampler),
            },
        ],
    })
}

fn uniform_layout_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
