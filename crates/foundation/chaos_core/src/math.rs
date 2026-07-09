//! Vocabulaire mathématique du moteur — point de passage unique vers glam.
//! Tout le moteur importe ses types d'ici (`chaos_core::math::Vec3`) : si la
//! bibliothèque sous-jacente devait changer, un seul module serait touché.
//! Conventions d'autorité : `docs/architecture/math-conventions.md`.

pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// Axes du monde — les constantes officielles du repère de Chaos Engine :
/// main droite, +Y haut, -Z avant. Toute direction « avant/droite/haut »
/// du moteur dérive de ces trois constantes, jamais de littéraux.
pub mod world {
    use super::Vec3;

    pub const RIGHT: Vec3 = Vec3::X;
    pub const UP: Vec3 = Vec3::Y;
    pub const FORWARD: Vec3 = Vec3::NEG_Z;
}

/// Projections officielles du moteur : main droite, profondeur 0..1 (la
/// convention wgpu/DirectX — jamais les variantes OpenGL en -1..1).
/// Point de passage unique : le moteur n'appelle jamais glam directement.
pub mod projection {
    pub use glam::camera::rh::proj::directx::perspective;
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    fn nearly(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn the_basis_is_right_handed() {
        assert!(nearly(world::RIGHT.cross(world::UP), -world::FORWARD));
        assert!(nearly(Vec3::X.cross(Vec3::Y), Vec3::Z));
    }

    #[test]
    fn matrices_are_column_major() {
        let translation = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(translation.w_axis, Vec4::new(1.0, 2.0, 3.0, 1.0));
    }

    #[test]
    fn positive_rotation_is_counterclockwise() {
        let quarter_turn = Quat::from_rotation_y(FRAC_PI_2);
        assert!(nearly(quarter_turn * Vec3::X, Vec3::NEG_Z));
    }

    #[test]
    fn blessed_projection_maps_depth_to_zero_one() {
        let perspective = projection::perspective(FRAC_PI_2, 16.0 / 9.0, 0.1, 100.0);
        let near_point = perspective.project_point3(Vec3::new(0.0, 0.0, -0.1));
        let far_point = perspective.project_point3(Vec3::new(0.0, 0.0, -100.0));
        assert!((near_point.z - 0.0).abs() < 1e-4);
        assert!((far_point.z - 1.0).abs() < 1e-4);
    }
}
