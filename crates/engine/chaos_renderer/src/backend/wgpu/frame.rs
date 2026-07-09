use chaos_core::ChaosResult;
use log::{debug, warn};

use crate::frame::{FramePlan, FrameSkipReason};

use crate::pool::PoolHandle;

use super::WgpuBackend;
use super::convert::{graphics_error, to_wgpu_color};

pub(super) enum Acquisition {
    Ready(wgpu::SurfaceTexture),
    Skip(FrameSkipReason),
}

impl WgpuBackend {
    pub(super) fn acquire_frame(&mut self) -> ChaosResult<Acquisition> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Ok(Acquisition::Ready(frame)),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                debug!("suboptimal surface texture, presenting anyway");
                Ok(Acquisition::Ready(frame))
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                warn!("surface lost or outdated, reconfiguring");
                self.surface.configure(&self.device, &self.config);
                Ok(Acquisition::Skip(FrameSkipReason::SurfaceReconfigured))
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                debug!("surface unavailable this frame, skipping");
                Ok(Acquisition::Skip(FrameSkipReason::SurfaceUnavailable))
            }
            wgpu::CurrentSurfaceTexture::Validation => Err(graphics_error(
                "validation error while acquiring the surface texture",
            )),
        }
    }

    pub(super) fn encode_frame(
        &self,
        view: &wgpu::TextureView,
        plan: &FramePlan,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chaos.frame"),
            });
        {
            let mut main_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chaos.main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(to_wgpu_color(plan.clear_color)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            main_pass.set_bind_group(0, &self.uniforms.frame_bind_group, &[]);
            let mut bound_pipeline = None;
            for (index, draw) in plan.draws.iter().enumerate() {
                let Some(pipeline) = self.pipelines.get(draw.pipeline.index()) else {
                    warn!("draw ignored: unknown pipeline {:?}", draw.pipeline);
                    continue;
                };
                let Some(object_bind_group) = self.uniforms.object_bind_group(index) else {
                    warn!("draw ignored: missing object uniform slot {index}");
                    continue;
                };
                if bound_pipeline != Some(draw.pipeline.index()) {
                    main_pass.set_pipeline(pipeline);
                    bound_pipeline = Some(draw.pipeline.index());
                }
                main_pass.set_bind_group(1, object_bind_group, &[]);
                if let Some(buffer_handle) = draw.vertex_buffer {
                    let pool_handle = PoolHandle {
                        index: buffer_handle.index,
                        generation: buffer_handle.generation,
                    };
                    let Some(buffer) = self.buffers.get(pool_handle) else {
                        warn!("draw ignored: stale vertex buffer {buffer_handle:?}");
                        continue;
                    };
                    main_pass.set_vertex_buffer(0, buffer.slice(..));
                }
                if let Some(buffer_handle) = draw.index_buffer {
                    let pool_handle = PoolHandle {
                        index: buffer_handle.index,
                        generation: buffer_handle.generation,
                    };
                    let Some(buffer) = self.buffers.get(pool_handle) else {
                        warn!("draw ignored: stale index buffer {buffer_handle:?}");
                        continue;
                    };
                    main_pass.set_index_buffer(buffer.slice(..), wgpu::IndexFormat::Uint16);
                    main_pass.draw_indexed(0..draw.element_count, 0, 0..1);
                } else {
                    main_pass.draw(0..draw.element_count, 0..1);
                }
            }
        }
        encoder.finish()
    }

    pub(super) fn submit_and_present(
        &self,
        frame: wgpu::SurfaceTexture,
        commands: wgpu::CommandBuffer,
    ) {
        self.queue.submit(Some(commands));
        self.queue.present(frame);
    }
}
