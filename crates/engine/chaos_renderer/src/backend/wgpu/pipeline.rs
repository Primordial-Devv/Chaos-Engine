use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::resources::{PipelineDescriptor, PipelineHandle, ShaderSource};

use super::WgpuBackend;
use super::convert::{
    to_wgpu_cull_mode, to_wgpu_depth_compare, to_wgpu_front_face, to_wgpu_step_mode,
    to_wgpu_topology, to_wgpu_vertex_attributes,
};
use super::depth::DEPTH_FORMAT;

/// Pipeline GPU accompagné de son contrat de binding : la passe doit savoir
/// si le groupe(2) material est attendu par les draws.
pub(super) struct PipelineEntry {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) uses_material: bool,
}

impl WgpuBackend {
    pub(super) fn build_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
        shader: &ShaderSource,
    ) -> ChaosResult<PipelineHandle> {
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let ShaderSource::Wgsl(source) = shader;
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&descriptor.label),
                source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
            });

        // La passe d'ombre binde le groupe(0) RÉDUIT (buffer frame seul) :
        // son pipeline doit viser le MÊME layout — la compatibilité des
        // bind groups est par slot, le groupe(1) objet reste le standard.
        let mut bind_group_layouts = if descriptor.depth_only {
            vec![
                Some(&self.uniforms.shadow_frame_layout),
                Some(&self.uniforms.object_layout),
            ]
        } else {
            vec![
                Some(&self.uniforms.frame_layout),
                Some(&self.uniforms.object_layout),
            ]
        };
        if descriptor.material {
            bind_group_layouts.push(Some(&self.material_bindings.layout));
        }
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&descriptor.label),
                bind_group_layouts: &bind_group_layouts,
                immediate_size: 0,
            });

        // Deux slots de vertex buffers au plus : le mesh (slot 0), puis
        // les données PAR INSTANCE (slot 1 — les permutations
        // instanciées).
        let vertex_attributes = descriptor
            .vertex_layout
            .as_ref()
            .map(to_wgpu_vertex_attributes);
        let instance_attributes = descriptor
            .instance_layout
            .as_ref()
            .map(to_wgpu_vertex_attributes);
        let mut vertex_buffers = Vec::new();
        if let (Some(layout), Some(attributes)) = (&descriptor.vertex_layout, &vertex_attributes) {
            vertex_buffers.push(Some(wgpu::VertexBufferLayout {
                array_stride: u64::from(layout.stride),
                step_mode: to_wgpu_step_mode(layout.step_mode),
                attributes,
            }));
        }
        if let (Some(layout), Some(attributes)) =
            (&descriptor.instance_layout, &instance_attributes)
        {
            vertex_buffers.push(Some(wgpu::VertexBufferLayout {
                array_stride: u64::from(layout.stride),
                step_mode: to_wgpu_step_mode(layout.step_mode),
                attributes,
            }));
        }

        let color_targets = [Some(wgpu::ColorTargetState {
            format: descriptor
                .color_target
                .map(super::convert::to_wgpu_texture_format)
                .unwrap_or(self.config.format),
            blend: Some(if descriptor.transparent {
                wgpu::BlendState::ALPHA_BLENDING
            } else {
                wgpu::BlendState::REPLACE
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&descriptor.label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(descriptor.vertex_entry.as_str()),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &vertex_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: to_wgpu_topology(descriptor.topology),
                    cull_mode: to_wgpu_cull_mode(descriptor.cull_mode),
                    front_face: to_wgpu_front_face(descriptor.front_face),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!descriptor.transparent),
                    depth_compare: Some(to_wgpu_depth_compare(descriptor.depth_compare)),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                // Profondeur seule : aucun étage fragment, aucune cible
                // couleur — le vertex shader suffit (les passes d'ombre).
                fragment: if descriptor.depth_only {
                    None
                } else {
                    Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(descriptor.fragment_entry.as_str()),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &color_targets,
                    })
                },
                multiview_mask: None,
                cache: None,
            });

        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "pipeline '{}' creation failed: {validation_error}",
                descriptor.label
            )));
        }

        let index = u32::try_from(self.pipelines.len())
            .map_err(|_| ChaosError::Graphics(String::from("pipeline capacity exceeded")))?;
        self.pipelines.push(PipelineEntry {
            pipeline,
            uses_material: descriptor.material,
        });
        let handle = PipelineHandle(index);
        debug!("pipeline '{}' created ({handle:?})", descriptor.label);
        Ok(handle)
    }
}
