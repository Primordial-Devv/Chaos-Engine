use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::pool::{PoolHandle, ResourcePool};
use crate::resources::{MaterialBindingDescriptor, MaterialBindingHandle};
use crate::shaders::inputs;

use super::WgpuBackend;
use super::convert::color_to_bytes;

const MATERIAL_UNIFORMS_SIZE: u64 = 16;

/// Mécanique du groupe(2) : le layout material standard (texture, sampler,
/// MaterialUniforms) et le pool des bind groups. Les groupes 0/1 (uniforms)
/// vivent dans `uniforms.rs` ; la convention de groupes est portée par
/// `shaders::inputs`.
pub(super) struct MaterialBindings {
    pub(super) layout: wgpu::BindGroupLayout,
    pool: ResourcePool<wgpu::BindGroup>,
}

impl MaterialBindings {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chaos.material_binding"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: inputs::MATERIAL_TEXTURE_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
            ],
        });
        Self {
            layout,
            pool: ResourcePool::new(),
        }
    }

    pub(super) fn get(&self, handle: MaterialBindingHandle) -> Option<&wgpu::BindGroup> {
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
        let texture = self
            .textures
            .get(PoolHandle {
                index: descriptor.texture.index,
                generation: descriptor.texture.generation,
            })
            .ok_or_else(|| {
                ChaosError::Graphics(format!(
                    "material binding '{}' refers to a stale or destroyed texture",
                    descriptor.label
                ))
            })?;
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&descriptor.label),
            size: MATERIAL_UNIFORMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&uniforms, 0, &color_to_bytes(descriptor.base_color));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&descriptor.label),
            layout: &self.material_bindings.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_TEXTURE_BINDING,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: inputs::MATERIAL_UNIFORMS_BINDING,
                    resource: uniforms.as_entire_binding(),
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
            .insert(bind_group)
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

    pub(super) fn release_material_binding(
        &mut self,
        handle: MaterialBindingHandle,
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.material_bindings.pool.remove(pool_handle) {
            Some(_bind_group) => {
                debug!("material binding released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "material binding handle is stale or already destroyed",
            ))),
        }
    }
}
