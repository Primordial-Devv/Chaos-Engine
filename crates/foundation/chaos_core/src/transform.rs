use crate::math::{Mat4, Quat, Vec3, world};

/// Transformation spatiale d'un objet : position, rotation, échelle.
///
/// Concept fondamental du moteur, réutilisé par l'ECS, les scènes,
/// l'éditeur, la physique et l'animation. Transformation locale uniquement :
/// la hiérarchie parent/enfant appartiendra au Scene System.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            rotation,
            ..Self::IDENTITY
        }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            scale,
            ..Self::IDENTITY
        }
    }

    pub fn with_translation(mut self, translation: Vec3) -> Self {
        self.translation = translation;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Matrice modèle (ordre TRS : échelle, puis rotation, puis translation).
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Direction avant locale (convention moteur : `math::world::FORWARD`).
    pub fn forward(&self) -> Vec3 {
        self.rotation * world::FORWARD
    }

    /// Direction droite locale (`math::world::RIGHT`).
    pub fn right(&self) -> Vec3 {
        self.rotation * world::RIGHT
    }

    /// Direction haut locale (`math::world::UP`).
    pub fn up(&self) -> Vec3 {
        self.rotation * world::UP
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    fn nearly(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn identity_produces_identity_matrix() {
        assert_eq!(Transform::IDENTITY.matrix(), Mat4::IDENTITY);
        assert_eq!(Transform::default(), Transform::IDENTITY);
    }

    #[test]
    fn translation_lands_in_the_matrix() {
        let transform = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let point = transform.matrix().transform_point3(Vec3::ZERO);
        assert!(nearly(point, Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn trs_order_scales_then_rotates_then_translates() {
        let transform = Transform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let point = transform.matrix().transform_point3(Vec3::X);
        assert!(nearly(point, Vec3::new(10.0, 0.0, -2.0)));
    }

    #[test]
    fn local_directions_follow_right_handed_convention() {
        let identity = Transform::IDENTITY;
        assert!(nearly(identity.forward(), Vec3::NEG_Z));
        assert!(nearly(identity.right(), Vec3::X));
        assert!(nearly(identity.up(), Vec3::Y));

        let quarter_turn = Transform::from_rotation(Quat::from_rotation_y(FRAC_PI_2));
        assert!(nearly(quarter_turn.forward(), Vec3::NEG_X));
        assert!(nearly(quarter_turn.right(), Vec3::NEG_Z));
        assert!(nearly(quarter_turn.up(), Vec3::Y));
    }

    #[test]
    fn non_uniform_scale_lands_in_the_matrix() {
        let transform = Transform::from_scale(Vec3::new(2.0, 3.0, 4.0));
        let point = transform.matrix().transform_point3(Vec3::ONE);
        assert!(nearly(point, Vec3::new(2.0, 3.0, 4.0)));
    }
}
