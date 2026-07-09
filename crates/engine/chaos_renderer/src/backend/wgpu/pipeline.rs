use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::resources::{PipelineDescriptor, PipelineHandle, ShaderSource};

use super::WgpuBackend;
use super::convert::{
    to_wgpu_cull_mode, to_wgpu_front_face, to_wgpu_step_mode, to_wgpu_topology,
    to_wgpu_vertex_attributes,
};

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

        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&descriptor.label),
                bind_group_layouts: &[],
                immediate_size: 0,
            });

        let vertex_attributes = descriptor
            .vertex_layout
            .as_ref()
            .map(to_wgpu_vertex_attributes);
        let vertex_buffers = match (&descriptor.vertex_layout, &vertex_attributes) {
            (Some(layout), Some(attributes)) => vec![Some(wgpu::VertexBufferLayout {
                array_stride: u64::from(layout.stride),
                step_mode: to_wgpu_step_mode(layout.step_mode),
                attributes,
            })],
            _ => Vec::new(),
        };

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
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(descriptor.fragment_entry.as_str()),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
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
        self.pipelines.push(pipeline);
        let handle = PipelineHandle(index);
        debug!("pipeline '{}' created ({handle:?})", descriptor.label);
        Ok(handle)
    }
}
