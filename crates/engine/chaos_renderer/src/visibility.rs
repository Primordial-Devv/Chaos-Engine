//! La VISIBILITÉ du renderer : le frustum d'une vue et son test
//! d'intersection avec des bounds monde — l'outil du frustum culling
//! (et du futur éditeur). Chaque passe possède SON frustum (celui de sa
//! caméra) ; la passe d'ombre teste celui de la LUMIÈRE — jamais le
//! frustum principal aveuglément.

use chaos_core::math::{Aabb, Mat4, Vec3, Vec4};

/// Le frustum d'une vue : les six plans extraits de sa matrice
/// vue-projection (méthode de Gribb-Hartmann, convention de profondeur
/// 0..1 — la nôtre). Chaque plan est un `Vec4` (normale xyz pointant
/// vers l'INTÉRIEUR, distance w), NON normalisé — le test ne lit que le
/// signe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extrait le frustum d'une matrice vue-projection — gauche, droite,
    /// bas, haut, près (z ≥ 0 en clip 0..1), loin.
    pub fn from_view_projection(view_projection: Mat4) -> Self {
        let row = |index: usize| {
            Vec4::new(
                view_projection.x_axis[index],
                view_projection.y_axis[index],
                view_projection.z_axis[index],
                view_projection.w_axis[index],
            )
        };
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        Self {
            planes: [w + x, w - x, w + y, w - y, z, w - z],
        }
    }

    /// La boîte TOUCHE-t-elle le frustum ? Test du p-vertex par plan,
    /// frontière INCLUSIVE (`>= 0`) — CONSERVATIF : une boîte même
    /// partiellement visible passe toujours, un faux rejet est
    /// impossible (le sur-dessin des coins du frustum est le prix V1).
    pub fn intersects(&self, bounds: &Aabb) -> bool {
        for plane in &self.planes {
            let positive = Vec3::new(
                if plane.x >= 0.0 {
                    bounds.max.x
                } else {
                    bounds.min.x
                },
                if plane.y >= 0.0 {
                    bounds.max.y
                } else {
                    bounds.min.y
                },
                if plane.z >= 0.0 {
                    bounds.max.z
                } else {
                    bounds.min.z
                },
            );
            if plane.x * positive.x + plane.y * positive.y + plane.z * positive.z + plane.w < 0.0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::math::projection;

    use super::*;

    fn unit_box_at(center: Vec3) -> Aabb {
        Aabb::from_points([center - Vec3::splat(0.5), center + Vec3::splat(0.5)]).unwrap()
    }

    #[test]
    fn an_orthographic_frustum_accepts_inside_and_rejects_outside() {
        // L'ortho bénie regarde -Z : ce qui vit dans la boîte passe,
        // ce qui la quitte est rejeté, la frontière est INCLUSIVE.
        let frustum = Frustum::from_view_projection(projection::orthographic(
            -10.0, 10.0, -10.0, 10.0, 0.1, 100.0,
        ));
        assert!(frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, -50.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(25.0, 0.0, -50.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(0.0, -25.0, -50.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, 50.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, -200.0))));
        // À cheval sur le plan droit : partiellement visible → JAMAIS
        // rejetée (le conservatisme est le contrat).
        assert!(frustum.intersects(&unit_box_at(Vec3::new(10.0, 0.0, -50.0))));
    }

    #[test]
    fn a_perspective_frustum_rejects_what_lives_behind_the_camera() {
        let frustum = Frustum::from_view_projection(projection::perspective(
            std::f32::consts::FRAC_PI_2,
            1.0,
            0.1,
            100.0,
        ));
        assert!(frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, -10.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, 10.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(100.0, 0.0, -10.0))));
    }

    #[test]
    fn the_mock_bench_camera_sees_a_thousand_units() {
        // La caméra LARGE du banc d'essai mock : une échelle pure —
        // w_axis nul (les assertions exactes), ±1000 visibles, z
        // ARRIÈRE couvert.
        let frustum =
            Frustum::from_view_projection(Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)));
        assert!(frustum.intersects(&unit_box_at(Vec3::new(500.0, -500.0, -500.0))));
        assert!(frustum.intersects(&unit_box_at(Vec3::ZERO)));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(1500.0, 0.0, -500.0))));
        assert!(!frustum.intersects(&unit_box_at(Vec3::new(0.0, 0.0, -1500.0))));
    }
}
