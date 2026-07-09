use std::f32::consts::FRAC_PI_3;

use crate::math::{Mat4, projection};
use crate::transform::Transform;

/// Projection perspective : FOV vertical (radians), aspect ratio, plans
/// near/far. L'orthographique arrivera sous forme d'enum le moment venu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perspective {
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Perspective {
    fn default() -> Self {
        Self {
            fov_y: FRAC_PI_3,
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 1000.0,
        }
    }
}

/// Point de vue 3D du moteur : un Transform dans le monde + une projection.
///
/// Concept moteur pur (données + maths), consommé par le renderer via
/// `view_projection()` et destiné à l'ECS, aux scènes, à l'éditeur et aux
/// outils de debug. Le moteur n'impose aucune politique de caméra : qui la
/// possède et la pilote relève des systèmes de plus haut niveau.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Camera {
    pub transform: Transform,
    pub projection: Perspective,
}

impl Camera {
    /// Matrice vue : inverse de la matrice monde de la caméra
    /// (convention verrouillée dans `docs/architecture/math-conventions.md`).
    pub fn view_matrix(&self) -> Mat4 {
        self.transform.matrix().inverse()
    }

    /// Matrice projection — via la projection bénie du moteur
    /// (main droite, profondeur 0..1).
    pub fn projection_matrix(&self) -> Mat4 {
        projection::perspective(
            self.projection.fov_y,
            self.projection.aspect,
            self.projection.near,
            self.projection.far,
        )
    }

    /// Matrice vue-projection (`projection × view`), prête pour le renderer.
    pub fn view_projection(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Met à jour l'aspect ratio depuis une taille de surface ; une hauteur
    /// nulle (minimisation) est ignorée.
    pub fn set_viewport(&mut self, width: u32, height: u32) {
        if height == 0 {
            return;
        }
        self.projection.aspect = width as f32 / height as f32;
    }
}

#[cfg(test)]
mod tests {
    use crate::math::Vec3;

    use super::*;

    fn nearly(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn defaults_are_sane() {
        let camera = Camera::default();
        assert!(camera.projection.fov_y > 0.0);
        assert!(camera.projection.near < camera.projection.far);
        assert!(camera.projection.aspect > 0.0);
    }

    #[test]
    fn view_matrix_inverts_the_camera_transform() {
        let mut camera = Camera::default();
        camera.transform.translation = Vec3::new(1.0, 2.0, 5.0);
        let eye_in_view = camera
            .view_matrix()
            .transform_point3(camera.transform.translation);
        assert!(nearly(eye_in_view, Vec3::ZERO));
    }

    #[test]
    fn point_straight_ahead_projects_to_ndc_center() {
        let mut camera = Camera::default();
        camera.transform.translation = Vec3::new(0.0, 0.0, 2.0);
        let ndc = camera
            .view_projection()
            .project_point3(Vec3::new(0.0, 0.0, -3.0));
        assert!(ndc.x.abs() < 1e-4);
        assert!(ndc.y.abs() < 1e-4);
        assert!(ndc.z > 0.0 && ndc.z < 1.0);
    }

    #[test]
    fn point_to_the_right_lands_in_positive_x() {
        let camera = Camera::default();
        let ndc = camera
            .view_projection()
            .project_point3(Vec3::new(1.0, 0.0, -3.0));
        assert!(ndc.x > 0.0);
    }

    #[test]
    fn view_projection_composes_projection_then_view() {
        let mut camera = Camera::default();
        camera.transform.translation = Vec3::new(0.5, -1.0, 4.0);
        let expected = camera.projection_matrix() * camera.view_matrix();
        assert_eq!(camera.view_projection(), expected);
    }

    #[test]
    fn set_viewport_updates_aspect_and_ignores_zero_height() {
        let mut camera = Camera::default();
        camera.set_viewport(1920, 1080);
        assert!((camera.projection.aspect - 16.0 / 9.0).abs() < 1e-6);
        camera.set_viewport(800, 0);
        assert!((camera.projection.aspect - 16.0 / 9.0).abs() < 1e-6);
    }
}
