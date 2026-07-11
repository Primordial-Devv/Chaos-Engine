use chaos_core::{ChaosError, ChaosResult};
use log::debug;

use crate::resources::{SamplerDescriptor, SamplerHandle};

use crate::pool::PoolHandle;

use super::WgpuBackend;
use super::convert::{to_wgpu_address_mode, to_wgpu_filter_mode, to_wgpu_mipmap_filter_mode};

impl WgpuBackend {
    pub(super) fn build_sampler(
        &mut self,
        descriptor: &SamplerDescriptor,
    ) -> ChaosResult<SamplerHandle> {
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let address_mode = to_wgpu_address_mode(descriptor.address_mode);
        let filter = to_wgpu_filter_mode(descriptor.filter);
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&descriptor.label),
            address_mode_u: address_mode,
            address_mode_v: address_mode,
            address_mode_w: address_mode,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter: to_wgpu_mipmap_filter_mode(descriptor.mip_filter),
            anisotropy_clamp: descriptor.anisotropy,
            ..Default::default()
        });

        if let Some(validation_error) = pollster::block_on(error_scope.pop()) {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}' creation failed: {validation_error}",
                descriptor.label
            )));
        }

        let pool_handle = self
            .samplers
            .insert(sampler)
            .ok_or_else(|| ChaosError::Graphics(String::from("sampler pool capacity exceeded")))?;
        let handle = SamplerHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!(
            "sampler '{}' created ({:?}, {:?}, {handle:?})",
            descriptor.label, descriptor.filter, descriptor.address_mode
        );
        Ok(handle)
    }

    pub(super) fn release_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        match self.samplers.remove(pool_handle) {
            Some(_sampler) => {
                debug!("sampler released ({handle:?})");
                Ok(())
            }
            None => Err(ChaosError::Graphics(String::from(
                "sampler handle is stale or already destroyed",
            ))),
        }
    }
}
