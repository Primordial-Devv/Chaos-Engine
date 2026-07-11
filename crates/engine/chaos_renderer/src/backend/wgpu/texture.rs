use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::resources::{TextureDescriptor, TextureHandle, mip_dimensions};

use crate::pool::PoolHandle;

use super::WgpuBackend;
use super::convert::{texel_row_bytes, to_wgpu_texture_format, to_wgpu_texture_usages};

impl WgpuBackend {
    pub(super) fn build_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> ChaosResult<TextureHandle> {
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let layers = descriptor.layer_count();
        let levels = descriptor.mip_level_count();
        let size = wgpu::Extent3d {
            width: descriptor.width,
            height: descriptor.height,
            depth_or_array_layers: layers,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&descriptor.label),
            size,
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: to_wgpu_texture_format(descriptor.format),
            usage: to_wgpu_texture_usages(descriptor.usage),
            view_formats: &[],
        });
        if !descriptor.pixels.is_empty() {
            // Le layout niveau-majeur : un write_texture par niveau,
            // toutes les couches d'un coup (rows_per_image découpe).
            let mut offset = 0usize;
            for level in 0..levels {
                let (level_width, level_height) =
                    mip_dimensions(descriptor.width, descriptor.height, level);
                let level_bytes = level_width as usize
                    * level_height as usize
                    * descriptor.format.bytes_per_pixel() as usize
                    * layers as usize;
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &descriptor.pixels[offset..offset + level_bytes],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(texel_row_bytes(
                            level_width,
                            descriptor.format.bytes_per_pixel(),
                        )),
                        rows_per_image: Some(level_height),
                    },
                    wgpu::Extent3d {
                        width: level_width,
                        height: level_height,
                        depth_or_array_layers: layers,
                    },
                );
                offset += level_bytes;
            }
        }

        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "texture '{}' creation failed: {validation_error}",
                descriptor.label
            )));
        }

        let pool_handle = self
            .textures
            .insert(texture)
            .ok_or_else(|| ChaosError::Graphics(String::from("texture pool capacity exceeded")))?;
        let handle = TextureHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!(
            "texture '{}' created ({}x{}, {:?}, {} bytes, {handle:?})",
            descriptor.label,
            descriptor.width,
            descriptor.height,
            descriptor.format,
            descriptor.pixels.len()
        );
        Ok(handle)
    }

    pub(super) fn write_texture_pixels(
        &mut self,
        handle: TextureHandle,
        pixels: &[u8],
    ) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(texture) = self.textures.get(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            )));
        };
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = texture.size();
        let bytes_per_pixel = texture.format().block_copy_size(None).unwrap_or(4);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texel_row_bytes(size.width, bytes_per_pixel)),
                rows_per_image: Some(size.height),
            },
            size,
        );
        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "texture update failed: {validation_error}"
            )));
        }
        debug!("texture updated ({handle:?}, {} bytes)", pixels.len());
        Ok(())
    }

    pub(super) fn release_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.textures.remove(pool_handle) {
            Some(_texture) => {
                debug!("texture released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            ))),
        }
    }
}
