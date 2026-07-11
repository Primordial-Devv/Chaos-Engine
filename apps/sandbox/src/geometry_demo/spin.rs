//! Le COMPORTEMENT de la démo : la rotation du cube central, en données
//! ECS — le contenu définit ses composants, le moteur fournit le
//! mécanisme.

use chaos_engine::{ChaosResult, Component, System, Time, Transform, World, math::Quat};

/// La rotation du cube central, en DONNÉES — le contenu définit ses
/// composants, le moteur fournit le mécanisme.
pub(super) struct Spin {
    pub(super) yaw_rate: f32,
    pub(super) pitch_rate: f32,
}

impl Component for Spin {}

/// Le système du spin : lit le temps du monde, oriente tout porteur de
/// `Spin`. Il vit dans l'app, pas dans le moteur — l'API d'un futur
/// gamemode.
pub(super) struct SpinSystem;

impl System for SpinSystem {
    fn name(&self) -> &str {
        "demo.spin"
    }

    fn run(&self, world: &mut World) -> ChaosResult<()> {
        let elapsed = world
            .resource::<Time>()
            .map(|time| time.elapsed.as_secs_f32())
            .unwrap_or_default();
        for (_, transform, spin) in world.query2_mut::<Transform, Spin>()? {
            transform.rotation = Quat::from_rotation_y(spin.yaw_rate * elapsed)
                * Quat::from_rotation_x(spin.pitch_rate * elapsed);
        }
        Ok(())
    }
}
