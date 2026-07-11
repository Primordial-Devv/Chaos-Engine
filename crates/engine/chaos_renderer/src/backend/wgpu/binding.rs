use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::pool::{PoolHandle, ResourcePool};
use crate::resources::{MaterialBindingDescriptor, MaterialBindingHandle, MaterialParams};
use crate::shaders::inputs;

use super::WgpuBackend;
use super::convert::material_params_to_bytes;

/// Les paramètres material : base_color + (metallic, roughness) +
/// émissif — trois vec4, le miroir de `MaterialUniforms` dans les WGSL.
const MATERIAL_UNIFORMS_SIZE: u64 = 48;

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Un binding material vivant : le bind group ET son buffer d'uniforms —
/// le buffer est RETENU pour que la mise à jour des paramètres écrive en
/// place (`write_buffer`), sans jamais recréer le bind group.
pub(super) struct MaterialBindingEntry {
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
}

/// Mécanique du groupe(2) : le layout material standard (texture, sampler,
/// MaterialUniforms) et le pool des bindings. Les groupes 0/1 (uniforms)
/// vivent dans `uniforms.rs` ; la convention de groupes est portée par
/// `shaders::inputs`.
pub(super) struct MaterialBindings {
    pub(super) layout: wgpu::BindGroupLayout,
    pool: ResourcePool<MaterialBindingEntry>,
}

impl MaterialBindings {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chaos.material_binding"),
            entries: &[
                texture_layout_entry(inputs::MATERIAL_TEXTURE_BINDING),
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::MATERIAL_SAMPLER_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::MATERIAL_UNIFORMS_BINDING,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(MATERIAL_UNIFORMS_SIZE),
                    },
                    count: None,
                },
                texture_layout_entry(inputs::MATERIAL_METALLIC_ROUGHNESS_BINDING),
                texture_layout_entry(inputs::MATERIAL_NORMAL_BINDING),
                texture_layout_entry(inputs::MATERIAL_OCCLUSION_BINDING),
                texture_layout_entry(inputs::MATERIAL_EMISSIVE_BINDING),
            ],
        });
        Self {
            layout,
            pool: ResourcePool::new(),
        }
    }

    pub(super) fn get(&self, handle: MaterialBindingHandle) -> Option<&wgpu::BindGroup> {
        self.pool
            .get(PoolHandle {
                index: handle.index,
                generation: handle.generation,
            })
            .map(|entry| &entry.bind_group)
    }

    fn entry(&self, handle: MaterialBindingHandle) -> Option<&MaterialBindingEntry> {
        self.pool.get(PoolHandle {
            index: handle.index,
            generation: handle.generation,
        })
    }
}

impl WgpuBackend {
    pub(super) fn build_material_binding(
        &mut self,
        descriptor: &MaterialBindingDescriptor,
    ) -> ChaosResult<MaterialBindingHandle> {
        let slots = [
            descriptor.texture,
            descriptor.metallic_roughness_texture,
            descriptor.normal_map,
            descriptor.occlusion_texture,
            descriptor.emissive_texture,
        ];
        let mut views = Vec::with_capacity(slots.len());
        for handle in slots {
            let texture = self
                .textures
                .get(PoolHandle {
                    index: handle.index,
                    generation: handle.generation,
                })
                .ok_or_else(|| {
                    ChaosError::Graphics(format!(
                        "material binding '{}' refers to a stale or destroyed texture",
                        descriptor.label
                    ))
                })?;
            views.push(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        }
        let sampler = self
            .samplers
            .get(PoolHandle {
                index: descriptor.sampler.index,
                generation: descriptor.sampler.generation,
            })
            .ok_or_else(|| {
                ChaosError::Graphics(format!(
                    "material binding '{}' refers to a stale or destroyed sampler",
                    descriptor.label
                ))
            })?;

        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&descriptor.label),
            size: MATERIAL_UNIFORMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&uniforms, 0, &material_params_to_bytes(&descriptor.params));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&descriptor.label),
            layout: &self.material_bindings.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_TEXTURE_BINDING,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_UNIFORMS_BINDING,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_METALLIC_ROUGHNESS_BINDING,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_NORMAL_BINDING,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_OCCLUSION_BINDING,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_EMISSIVE_BINDING,
                    resource: wgpu::BindingResource::TextureView(&views[4]),
                },
            ],
        });
        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "material binding '{}' creation failed: {validation_error}",
                descriptor.label
            )));
        }

        let pool_handle = self
            .material_bindings
            .pool
            .insert(MaterialBindingEntry {
                bind_group,
                uniforms,
            })
            .ok_or_else(|| {
                ChaosError::Graphics(String::from("material binding pool capacity exceeded"))
            })?;
        let handle = MaterialBindingHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!(
            "material binding '{}' created ({handle:?})",
            descriptor.label
        );
        Ok(handle)
    }

    /// Écrit les paramètres d'un binding vivant EN PLACE — le buffer
    /// d'uniforms est retenu, le bind group survit tel quel.
    pub(super) fn write_material_uniforms(
        &mut self,
        handle: MaterialBindingHandle,
        params: &MaterialParams,
    ) -> ChaosResult<()> {
        let Some(entry) = self.material_bindings.entry(handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material binding handle is stale or already destroyed",
            )));
        };
        self.queue
            .write_buffer(&entry.uniforms, 0, &material_params_to_bytes(params));
        Ok(())
    }

    pub(super) fn release_material_binding(
        &mut self,
        handle: MaterialBindingHandle,
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.material_bindings.pool.remove(pool_handle) {
            Some(_entry) => {
                debug!("material binding released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "material binding handle is stale or already destroyed",
            ))),
        }
    }
}
