use chaos_core::{ChaosError, ChaosResult};
use log::{debug, info};

use crate::capabilities::{
    CapabilityDecision, CapabilityStatus, DeviceLimits, RendererCapabilities,
};
use crate::config::RendererConfig;
use crate::target::SurfaceTarget;

use super::convert::{TargetHandles, graphics_error};

pub(super) struct GpuContext {
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) config: wgpu::SurfaceConfiguration,
    pub(super) description: String,
    /// La période des timestamps GPU (ns/tick) — `None` si l'adaptateur
    /// n'offre pas les timestamp queries : le temps GPU sera
    /// EXPLICITEMENT indisponible, jamais inventé.
    pub(super) timestamp_period: Option<f32>,
    /// Le rapport des capacités : ce qui a été détecté, décidé et
    /// pourquoi — capturé ICI, à la source des choix.
    pub(super) capabilities: RendererCapabilities,
}

/// Les limites ACCORDÉES au device, traduites en vocabulaire Chaos — la
/// borne que le Renderer fait respecter avant le backend.
fn device_limits(limits: &wgpu::Limits) -> DeviceLimits {
    DeviceLimits {
        max_texture_2d: limits.max_texture_dimension_2d,
        max_buffer_bytes: limits.max_buffer_size,
        max_bind_groups: limits.max_bind_groups,
        max_sampled_textures_per_stage: limits.max_sampled_textures_per_shader_stage,
        max_samplers_per_stage: limits.max_samplers_per_shader_stage,
        max_color_attachments: limits.max_color_attachments,
        uniform_offset_alignment: limits.min_uniform_buffer_offset_alignment,
        max_anisotropy: 16,
    }
}

fn decision(
    domain: &str,
    status: CapabilityStatus,
    detail: impl Into<String>,
) -> CapabilityDecision {
    CapabilityDecision {
        domain: String::from(domain),
        status,
        detail: detail.into(),
    }
}

pub(super) fn initialize(
    target: Box<dyn SurfaceTarget>,
    renderer_config: RendererConfig,
) -> ChaosResult<GpuContext> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

    let surface = instance
        .create_surface(TargetHandles(target))
        .map_err(graphics_error)?;

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(graphics_error)?;

    let adapter_info = adapter.get_info();
    let description = format!("wgpu ({} / {:?})", adapter_info.name, adapter_info.backend);
    info!("graphics adapter selected: {description}");

    // Les timestamp queries se demandent SEULEMENT si l'adaptateur les
    // offre — sinon le temps GPU est explicitement indisponible.
    let timestamps_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: if timestamps_supported {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        },
        ..Default::default()
    }))
    .map_err(graphics_error)?;
    let timestamp_period = timestamps_supported.then(|| queue.get_timestamp_period());
    match timestamp_period {
        Some(period) => debug!("GPU timestamps available ({period} ns/tick)"),
        None => debug!("GPU timestamps unavailable on this adapter"),
    }

    let surface_capabilities = surface.get_capabilities(&adapter);
    let srgb_format = surface_capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb);
    let format = srgb_format
        .or_else(|| surface_capabilities.formats.first().copied())
        .ok_or_else(|| ChaosError::Graphics(String::from("no compatible surface format")))?;

    let present_mode = if renderer_config.vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: renderer_config.width.max(1),
        height: renderer_config.height.max(1),
        present_mode,
        alpha_mode: surface_capabilities
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: Vec::new(),
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);
    debug!(
        "surface configured: {}x{} ({format:?})",
        config.width, config.height
    );

    // LE rapport des capacités : chaque domaine détecté, décidé et
    // EXPLIQUÉ — aucune hypothèse implicite. Les limites RESPECTÉES
    // sont celles ACCORDÉES au device (les défauts WebGPU demandés
    // délibérément : le plancher portable — l'élévation ciblée est
    // l'extension notée) ; le cœur WebGPU (cubemaps, comparison
    // samplers, Rgba16Float, Depth32Float) est garanti par contrat sur
    // Metal, DX12 et Vulkan — la garantie devient DITE.
    let granted = device.limits();
    let capabilities = RendererCapabilities {
        backend: format!("{:?}", adapter_info.backend),
        adapter: adapter_info.name.clone(),
        limits: device_limits(&granted),
        decisions: vec![
            if timestamps_supported {
                decision(
                    "timestamp queries",
                    CapabilityStatus::Active,
                    "offered by the adapter — the GPU frame span is measured",
                )
            } else {
                decision(
                    "timestamp queries",
                    CapabilityStatus::Disabled {
                        reason: String::from("not offered by the adapter"),
                    },
                    "GPU time is reported unavailable, never invented",
                )
            },
            decision(
                "presentation",
                CapabilityStatus::Active,
                format!(
                    "{present_mode:?} chosen (vsync={}); offered: {:?} — the Auto modes always fall back cleanly",
                    renderer_config.vsync, surface_capabilities.present_modes
                ),
            ),
            if srgb_format.is_some() {
                decision(
                    "surface format",
                    CapabilityStatus::Active,
                    format!("{format:?} — sRGB preferred and offered"),
                )
            } else {
                decision(
                    "surface format",
                    CapabilityStatus::Fallback {
                        reason: String::from("no sRGB surface format offered"),
                    },
                    format!("{format:?} — the first offered format"),
                )
            },
            decision(
                "hdr",
                CapabilityStatus::Active,
                "Rgba16Float sampled & filterable — WebGPU core, guaranteed on Metal/DX12/Vulkan",
            ),
            decision(
                "cubemaps & mips",
                CapabilityStatus::Active,
                "cube textures are WebGPU core; mip chains are generated on the CPU (device-independent)",
            ),
            decision(
                "comparison samplers & shadow maps",
                CapabilityStatus::Active,
                "Depth32Float sampleable + LessEqual comparison sampling (hardware PCF) — WebGPU core",
            ),
            decision(
                "depth format",
                CapabilityStatus::Active,
                "Depth32Float — the portable reference depth format",
            ),
            decision(
                "anisotropy",
                CapabilityStatus::Active,
                "clamped to x16 — the WebGPU core ceiling",
            ),
            decision(
                "uniform alignment",
                CapabilityStatus::Active,
                format!(
                    "device requires {} B; per-draw slots use dedicated buffers (dynamic offsets are the noted optimisation)",
                    granted.min_uniform_buffer_offset_alignment
                ),
            ),
        ],
    };

    Ok(GpuContext {
        surface,
        device,
        queue,
        config,
        description,
        timestamp_period,
        capabilities,
    })
}
