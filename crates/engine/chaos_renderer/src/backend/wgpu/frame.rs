use chaos_core::ChaosResult;
use log::{debug, warn};

use crate::frame::{FramePass, FrameShadowPass, FrameSkipReason};
use crate::pass::PassLoad;

use crate::pool::PoolHandle;
use crate::shaders::inputs;

use super::WgpuBackend;
use super::convert::{graphics_error, to_wgpu_color};

pub(super) enum Acquisition {
    Ready(wgpu::SurfaceTexture),
    Skip(FrameSkipReason),
}

/// Les opérations de profondeur d'une passe, dérivées du plan entier —
/// les deux règles symétriques qui garantissent qu'une profondeur n'est
/// JAMAIS lue indéfinie :
/// - store : `Store` seulement si une passe ULTÉRIEURE de la même
///   destination arrive en `Keep` (elle dessinera par-dessus et doit
///   tester contre cette profondeur) ; `Discard` sinon — l'optimisation
///   des GPU tile-based conservée ;
/// - load : `Load` seulement si `Keep` ET qu'une passe ANTÉRIEURE de la
///   même destination existe (elle aura storé, par la règle ci-dessus) ;
///   `Clear(1.0)` sinon — `Keep` conserve la COULEUR, jamais un contenu
///   de profondeur indéfini.
pub(super) fn depth_operations(passes: &[FramePass], index: usize) -> wgpu::Operations<f32> {
    let destination = passes[index].destination;
    let later_keep = passes[index + 1..]
        .iter()
        .any(|pass| pass.destination == destination && pass.load == PassLoad::Keep);
    let earlier_same = passes[..index]
        .iter()
        .any(|pass| pass.destination == destination);
    let load = if passes[index].load == PassLoad::Keep && earlier_same {
        wgpu::LoadOp::Load
    } else {
        wgpu::LoadOp::Clear(1.0)
    };
    let store = if later_keep {
        wgpu::StoreOp::Store
    } else {
        wgpu::StoreOp::Discard
    };
    wgpu::Operations { load, store }
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

    pub(super) fn encode_pass(
        &self,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        pass: &FramePass,
        depth_ops: wgpu::Operations<f32>,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chaos.frame"),
            });
        {
            let color_load = match pass.load {
                PassLoad::Clear(color) => wgpu::LoadOp::Clear(to_wgpu_color(color)),
                PassLoad::Keep => wgpu::LoadOp::Load,
            };
            let mut main_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass.label.as_str()),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: color_load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(depth_ops),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamps,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            main_pass.set_bind_group(inputs::FRAME_GROUP, &self.uniforms.frame_bind_group, &[]);
            let mut bound_pipeline = None;
            let mut bound_material = None;
            for (index, draw) in pass.draws.iter().enumerate() {
                let Some(entry) = self.pipelines.get(draw.pipeline.index()) else {
                    warn!("draw ignored: unknown pipeline {:?}", draw.pipeline);
                    continue;
                };
                let Some(object_bind_group) = self.uniforms.object_bind_group(index) else {
                    warn!("draw ignored: missing object uniform slot {index}");
                    continue;
                };
                if bound_pipeline != Some(draw.pipeline.index()) {
                    main_pass.set_pipeline(&entry.pipeline);
                    bound_pipeline = Some(draw.pipeline.index());
                }
                main_pass.set_bind_group(inputs::OBJECT_GROUP, object_bind_group, &[]);
                if entry.uses_material {
                    let Some(binding_handle) = draw.binding else {
                        warn!(
                            "draw ignored: pipeline {:?} expects material resources",
                            draw.pipeline
                        );
                        continue;
                    };
                    if bound_material != Some(binding_handle) {
                        let Some(bind_group) = self.material_bindings.get(binding_handle) else {
                            warn!("draw ignored: stale material binding {binding_handle:?}");
                            continue;
                        };
                        main_pass.set_bind_group(inputs::MATERIAL_GROUP, bind_group, &[]);
                        bound_material = Some(binding_handle);
                    }
                }
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
                // Un draw INSTANCIÉ binde sa tranche de l'instance
                // buffer au slot 1 et dessine sa plage — le classique
                // reste une instance unique.
                let instance_range = match draw.instances {
                    Some(range) => {
                        main_pass.set_vertex_buffer(1, self.instances.slice_from(range.first));
                        0..range.count
                    }
                    None => 0..1,
                };
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
                    main_pass.draw_indexed(0..draw.element_count, 0, instance_range);
                } else {
                    main_pass.draw(0..draw.element_count, instance_range);
                }
            }
            // Les batches de DEBUG s'encodent APRÈS tous les draws de la
            // passe (opaques → masked → ciel → transparents → debug),
            // l'overlay en dernier. Leur slot d'objet suit ceux des
            // draws (écrit à l'identité — le shader debug ne le lit
            // pas, le layout standard l'exige).
            for (offset, batch) in pass.debug.iter().enumerate() {
                let Some(entry) = self.pipelines.get(batch.pipeline.index()) else {
                    warn!("debug batch ignored: unknown pipeline {:?}", batch.pipeline);
                    continue;
                };
                let slot = pass.draws.len() + offset;
                let Some(object_bind_group) = self.uniforms.object_bind_group(slot) else {
                    warn!("debug batch ignored: missing object uniform slot {slot}");
                    continue;
                };
                if bound_pipeline != Some(batch.pipeline.index()) {
                    main_pass.set_pipeline(&entry.pipeline);
                    bound_pipeline = Some(batch.pipeline.index());
                }
                main_pass.set_bind_group(inputs::OBJECT_GROUP, object_bind_group, &[]);
                main_pass.set_vertex_buffer(0, self.debug_vertices.slice_from(batch.first_vertex));
                main_pass.draw(0..batch.vertex_count, 0..1);
            }
        }
        encoder.finish()
    }

    /// Encode la passe d'OMBRE : profondeur seule (aucun attachement
    /// couleur), clear à 1.0 et `Store` — la map doit SURVIVRE à la
    /// passe pour être échantillonnée par les suivantes (jamais dérivée
    /// de `depth_operations`, qui raisonne sur les destinations du
    /// plan). Le groupe(0) est le layout RÉDUIT (buffer frame seul) :
    /// le groupe complet porterait la map en texture alors qu'elle est
    /// l'attachement — le conflit d'usage wgpu. Zéro clear = une map
    /// entièrement à 1.0 : aucune ombre fantôme d'une frame passée.
    pub(super) fn encode_shadow_pass(
        &self,
        view: &wgpu::TextureView,
        shadow: &FrameShadowPass,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("chaos.shadow"),
            });
        {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chaos.shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamps,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            shadow_pass.set_bind_group(
                inputs::FRAME_GROUP,
                &self.uniforms.shadow_frame_bind_group,
                &[],
            );
            let mut bound_pipeline = None;
            for (index, draw) in shadow.draws.iter().enumerate() {
                let Some(entry) = self.pipelines.get(draw.pipeline.index()) else {
                    warn!("shadow draw ignored: unknown pipeline {:?}", draw.pipeline);
                    continue;
                };
                let Some(object_bind_group) = self.uniforms.object_bind_group(index) else {
                    warn!("shadow draw ignored: missing object uniform slot {index}");
                    continue;
                };
                if bound_pipeline != Some(draw.pipeline.index()) {
                    shadow_pass.set_pipeline(&entry.pipeline);
                    bound_pipeline = Some(draw.pipeline.index());
                }
                shadow_pass.set_bind_group(inputs::OBJECT_GROUP, object_bind_group, &[]);
                if let Some(buffer_handle) = draw.vertex_buffer {
                    let pool_handle = PoolHandle {
                        index: buffer_handle.index,
                        generation: buffer_handle.generation,
                    };
                    let Some(buffer) = self.buffers.get(pool_handle) else {
                        warn!("shadow draw ignored: stale vertex buffer {buffer_handle:?}");
                        continue;
                    };
                    shadow_pass.set_vertex_buffer(0, buffer.slice(..));
                }
                // Les casters instanciés suivent la même mécanique que
                // les passes couleur : tranche au slot 1, plage 0..n.
                let instance_range = match draw.instances {
                    Some(range) => {
                        shadow_pass.set_vertex_buffer(1, self.instances.slice_from(range.first));
                        0..range.count
                    }
                    None => 0..1,
                };
                if let Some(buffer_handle) = draw.index_buffer {
                    let pool_handle = PoolHandle {
                        index: buffer_handle.index,
                        generation: buffer_handle.generation,
                    };
                    let Some(buffer) = self.buffers.get(pool_handle) else {
                        warn!("shadow draw ignored: stale index buffer {buffer_handle:?}");
                        continue;
                    };
                    shadow_pass.set_index_buffer(buffer.slice(..), wgpu::IndexFormat::Uint16);
                    shadow_pass.draw_indexed(0..draw.element_count, 0, instance_range);
                } else {
                    shadow_pass.draw(0..draw.element_count, instance_range);
                }
            }
        }
        encoder.finish()
    }
}
