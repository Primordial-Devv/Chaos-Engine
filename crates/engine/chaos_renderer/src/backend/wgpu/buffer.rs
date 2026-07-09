use chaos_core::{ChaosError, ChaosResult};
use log::debug;
use wgpu::util::DeviceExt;

use crate::resources::{BufferDescriptor, BufferHandle, BufferKind};

use crate::pool::PoolHandle;

use super::WgpuBackend;

impl WgpuBackend {
    pub(super) fn build_buffer(
        &mut self,
        descriptor: &BufferDescriptor,
    ) -> ChaosResult<BufferHandle> {
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let usage = match descriptor.kind {
            BufferKind::Vertex => wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            BufferKind::Index => wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        };
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&descriptor.label),
                contents: &descriptor.contents,
                usage,
            });

        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "buffer '{}' creation failed: {validation_error}",
                descriptor.label
            )));
        }

        let pool_handle = self
            .buffers
            .insert(buffer)
            .ok_or_else(|| ChaosError::Graphics(String::from("buffer pool capacity exceeded")))?;
        let handle = BufferHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!(
            "buffer '{}' created ({} bytes, {handle:?})",
            descriptor.label,
            descriptor.contents.len()
        );
        Ok(handle)
    }

    pub(super) fn release_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.buffers.remove(pool_handle) {
            Some(_buffer) => {
                debug!("buffer released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "buffer handle is stale or already destroyed",
            ))),
        }
    }
}
