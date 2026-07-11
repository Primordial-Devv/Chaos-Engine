//! Les tests white-box du renderer, découpés par domaine dans l'ordre
//! historique des sous-phases (ressources → passes → éclairage → ombres →
//! instancing → culling → debug → diagnostics) ; les helpers partagés
//! vivent dans `support`.

use chaos_core::Transform;
use chaos_core::math::{Vec3, projection};

use crate::debug::DEFAULT_DEBUG_CATEGORY;
use crate::frame::FrameSkipReason;
use crate::resources::{
    SamplerAddressMode, SamplerFilter, ShaderSource, TextureFormat, TextureMips,
};
use crate::shaders::builtin;
use crate::shadow::ShadowVolume;
use crate::testing::{
    Journal, create_pipeline_lines, mock_renderer, mock_renderer_with, mock_renderer_with_limits,
    render_lines, set_shadow_lines, shadow_lines,
};

use super::*;

mod support;
use support::*;

mod core;
mod culling;
mod debug;
mod diagnostics;
mod instancing;
mod lifetime;
mod lighting;
mod materials;
mod opacity;
mod passes;
mod shadows;
mod targets;
mod textures;
