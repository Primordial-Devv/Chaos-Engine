use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::pool::PoolHandle;
use crate::resources::{
    RenderTargetDescriptor, RenderTargetHandle, TextureDescriptor, TextureHandle,
};

use super::WgpuBackend;
use super::depth;

/// Une cible hors écran côté backend : sa couleur vit DANS le pool de
/// textures (bindable par les materials, retirée par le chemin texture
/// habituel), ses vues et sa profondeur propre vivent ici.
pub(super) struct RenderTargetRecord {
    pub(super) color_view: wgpu::TextureView,
    pub(super) depth_view: wgpu::TextureView,
}

impl WgpuBackend {
    pub(super) fn build_render_target(
        &mut self,
        descriptor: &RenderTargetDescriptor,
    ) -> ChaosResult<(RenderTargetHandle, TextureHandle)> {
        let color = self.build_texture(&TextureDescriptor::render_target(
            descriptor.label.clone(),
            descriptor.width,
            descriptor.height,
            descriptor.format,
        ))?;
        let color_pool_handle = PoolHandle {
            index: color.index,
            generation: color.generation,
        };
        let Some(color_texture) = self.textures.get(color_pool_handle) else {
            return Err(ChaosError::Graphics(format!(
                "render target '{}': its color texture vanished at creation",
                descriptor.label
            )));
        };
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view =
            depth::create_depth_view(&self.device, descriptor.width, descriptor.height);
        let record = RenderTargetRecord {
            color_view,
            depth_view,
        };
        let pool_handle = self.render_targets.insert(record).ok_or_else(|| {
            ChaosError::Graphics(String::from("render target pool capacity exceeded"))
        })?;
        let handle = RenderTargetHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!(
            "render target '{}' created ({}x{}, {:?}, {handle:?})",
            descriptor.label, descriptor.width, descriptor.height, descriptor.format
        );
        Ok((handle, color))
    }

    pub(super) fn release_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.render_targets.remove(pool_handle) {
            Some(_record) => {
                debug!("render target released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "render target handle is stale or already destroyed",
            ))),
        }
    }
}
