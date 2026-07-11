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
    pub use glam::camera::rh::proj::directx::{orthographic, perspective};
}

/// Vues officielles du moteur : main droite, +Y haut. `look_to` construit
/// la matrice de vue depuis une position et une DIRECTION de regard —
/// direction et up doivent être NORMALISÉS (le contrat glam).
pub mod view {
    pub use glam::camera::rh::view::look_to_mat4 as look_to;
}

/// Une boîte englobante alignée sur les axes (AABB) — le vocabulaire
/// commun des BOUNDS du moteur : le renderer s'en sert pour la
/// visibilité (frustum culling), la physique s'en servira pour ses
/// phases larges. Une `Aabb` construite par [`Aabb::from_points`] est
/// TOUJOURS valide (finie, min ≤ max) — les données dégénérées sont
/// refusées à la source, jamais transportées.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Le coin minimal de la boîte.
    pub min: Vec3,
    /// Le coin maximal de la boîte.
    pub max: Vec3,
}

impl Aabb {
    /// La boîte englobante d'un nuage de points — `None` si le nuage est
    /// VIDE ou porte une coordonnée non finie : un bound invalide
    /// n'existe jamais.
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut bounds: Option<Self> = None;
        for point in points {
            if !point.is_finite() {
                return None;
            }
            bounds = Some(match bounds {
                Some(current) => Self {
                    min: current.min.min(point),
                    max: current.max.max(point),
                },
                None => Self {
                    min: point,
                    max: point,
                },
            });
        }
        bounds
    }

    /// Le centre de la boîte.
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Les demi-étendues de la boîte.
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// La boîte transformée dans un autre espace (méthode d'Arvo) : le
    /// centre passe par la matrice, les demi-étendues par sa valeur
    /// ABSOLUE — le résultat englobe la boîte tournée : CONSERVATIF
    /// (jamais plus petit que la vraie empreinte), le contrat du culling.
    pub fn transformed(&self, matrix: Mat4) -> Self {
        let center = matrix.transform_point3(self.center());
        let extents = self.half_extents();
        let absolute = Vec3::new(
            matrix.x_axis.x.abs() * extents.x
                + matrix.y_axis.x.abs() * extents.y
                + matrix.z_axis.x.abs() * extents.z,
            matrix.x_axis.y.abs() * extents.x
                + matrix.y_axis.y.abs() * extents.y
                + matrix.z_axis.y.abs() * extents.z,
            matrix.x_axis.z.abs() * extents.x
                + matrix.y_axis.z.abs() * extents.y
                + matrix.z_axis.z.abs() * extents.z,
        );
        Self {
            min: center - absolute,
            max: center + absolute,
        }
    }
}

/// La matrice de transformation des NORMALES d'un modèle : l'inverse
/// transposée de son 3×3 — l'échelle non uniforme casserait la
/// perpendicularité si le modèle était appliqué tel quel. Une matrice
/// singulière (échelle nulle) rend l'IDENTITÉ, jamais des NaN. La normale
/// transformée doit être renormalisée par le consommateur (une échelle
/// uniforme change sa longueur).
pub fn normal_matrix(model: Mat4) -> Mat4 {
    let linear = Mat3::from_mat4(model);
    if linear.determinant().abs() < f32::EPSILON {
        return Mat4::IDENTITY;
    }
    Mat4::from_mat3(linear.inverse().transpose())
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

    #[test]
    fn blessed_orthographic_maps_depth_to_zero_one() {
        let orthographic = projection::orthographic(-2.0, 2.0, -1.0, 1.0, 0.1, 100.0);
        let near_point = orthographic.project_point3(Vec3::new(0.0, 0.0, -0.1));
        let far_point = orthographic.project_point3(Vec3::new(0.0, 0.0, -100.0));
        assert!((near_point.z - 0.0).abs() < 1e-5);
        assert!((far_point.z - 1.0).abs() < 1e-5);
        let corner = orthographic.project_point3(Vec3::new(2.0, 1.0, -0.1));
        assert!(nearly(corner, Vec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn look_to_faces_the_blessed_forward() {
        let view = view::look_to(Vec3::ZERO, world::FORWARD, world::UP);
        assert_eq!(view, Mat4::IDENTITY);
        let eye = Vec3::new(0.0, 0.0, 5.0);
        let ahead =
            view::look_to(eye, world::FORWARD, world::UP).project_point3(Vec3::new(0.0, 0.0, 2.0));
        assert!(nearly(ahead, Vec3::new(0.0, 0.0, -3.0)));
    }

    #[test]
    fn aabb_from_points_refuses_the_degenerate() {
        assert!(Aabb::from_points([]).is_none());
        assert!(Aabb::from_points([Vec3::new(f32::NAN, 0.0, 0.0)]).is_none());
        assert!(Aabb::from_points([Vec3::ZERO, Vec3::new(0.0, f32::INFINITY, 0.0)]).is_none());
        let single = Aabb::from_points([Vec3::ONE]).unwrap();
        assert_eq!(single.min, Vec3::ONE);
        assert_eq!(single.max, Vec3::ONE);
        let cloud = Aabb::from_points([
            Vec3::new(1.0, -2.0, 3.0),
            Vec3::new(-1.0, 2.0, 0.0),
            Vec3::new(0.5, 0.0, -3.0),
        ])
        .unwrap();
        assert_eq!(cloud.min, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(cloud.max, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn aabb_transformed_is_exact_in_translation_and_conservative_in_rotation() {
        let unit = Aabb::from_points([Vec3::splat(-1.0), Vec3::splat(1.0)]).unwrap();
        let moved = unit.transformed(Mat4::from_translation(Vec3::new(5.0, 0.0, -2.0)));
        assert!(nearly(moved.center(), Vec3::new(5.0, 0.0, -2.0)));
        assert!(nearly(moved.half_extents(), Vec3::ONE));
        // Sous rotation de 45°, la boîte d'Arvo ENGLOBE la vraie
        // empreinte (√2 sur les axes tournés) — jamais plus petite.
        let turned = unit.transformed(Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4));
        let expected = 2.0f32.sqrt();
        assert!((turned.half_extents().x - expected).abs() < 1e-5);
        assert!((turned.half_extents().z - expected).abs() < 1e-5);
        assert!((turned.half_extents().y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_normal_matrix_preserves_perpendicularity() {
        // Échelle non uniforme + rotation : le modèle brut casserait la
        // perpendicularité, l'inverse-transposée la conserve.
        let model = Mat4::from_rotation_z(0.7)
            * Mat4::from_scale(Vec3::new(8.0, 1.0, 0.5))
            * Mat4::from_rotation_x(-FRAC_PI_2);
        let normal = Vec3::Z;
        let tangent = Vec3::X;
        let transformed_normal = (normal_matrix(model) * normal.extend(0.0))
            .truncate()
            .normalize();
        let transformed_tangent = (model * tangent.extend(0.0)).truncate();
        assert!(transformed_normal.dot(transformed_tangent).abs() < 1e-5);
    }

    #[test]
    fn a_singular_model_falls_back_to_identity() {
        let flattened = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(normal_matrix(flattened), Mat4::IDENTITY);
        let transformed = normal_matrix(flattened) * Vec3::Y.extend(0.0);
        assert!(transformed.is_finite());
    }
}
