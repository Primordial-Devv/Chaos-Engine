//! L'ENVIRONNEMENT de la démo : la cubemap HDR procédurale (gradient +
//! soleil aligné sur la directionnelle) aux mips GÉNÉRÉES — la rugosité
//! IBL parcourt la chaîne, le ciel remplace le fond uni des passes
//! Clear (le miroir compris). E la bascule, V/B règlent l'exposition.

use chaos_engine::{
    ChaosResult, EnvironmentDescriptor, Renderer, TextureDescriptor, TextureFormat, TextureHandle,
    TextureMips,
};

use super::content::sky_cubemap_pixels;

/// Le rig d'environnement : la cubemap du ciel, le toggle E et
/// l'exposition réglée par V/B.
pub(super) struct Environment {
    sky: TextureHandle,
    disabled: bool,
    exposure: f32,
}

impl Environment {
    /// Crée la cubemap HDR (`Rgba16Float` 64×64, mips générées) et
    /// l'installe comme environnement actif, exposition 1.
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        let sky = renderer.create_texture(
            &TextureDescriptor::cube(
                "demo.sky",
                64,
                TextureFormat::Rgba16Float,
                sky_cubemap_pixels(64),
            )
            .with_mips(TextureMips::Generate),
        )?;
        renderer.set_environment(&EnvironmentDescriptor::new(sky))?;
        Ok(Self {
            sky,
            disabled: false,
            exposure: 1.0,
        })
    }

    /// E : bascule l'environnement (set/clear) — ciel et IBL
    /// disparaissent, le fond uni et l'ambiante plate reprennent : le
    /// chemin des fallbacks, exercé en réel.
    pub(super) fn toggle(&mut self, renderer: &mut Renderer) {
        self.disabled = !self.disabled;
        let toggle = if self.disabled {
            renderer.clear_environment()
        } else {
            renderer.set_environment(&EnvironmentDescriptor::new(self.sky))
        };
        match toggle {
            Ok(()) => log::info!(
                "environment {}",
                if self.disabled { "cleared" } else { "set" }
            ),
            Err(environment_error) => {
                log::warn!("environment toggle failed: {environment_error}");
            }
        }
    }

    /// V/B : règlent l'exposition globale (÷/× 1.25, bornée [0.25, 4]) —
    /// le ciel et les materials PBR s'exposent ensemble.
    pub(super) fn adjust_exposure(&mut self, renderer: &mut Renderer, factor: f32) {
        self.exposure = (self.exposure * factor).clamp(0.25, 4.0);
        match renderer.set_exposure(self.exposure) {
            Ok(()) => log::info!("exposure {:.2}", self.exposure),
            Err(exposure_error) => log::warn!("exposure update failed: {exposure_error}"),
        }
    }
}
