use chaos_core::math::{self, Mat4, Vec3};
use chaos_core::{ChaosError, ChaosResult};

/// La résolution minimale d'une shadow map.
pub const MIN_SHADOW_RESOLUTION: u32 = 16;

/// La résolution maximale d'une shadow map — la limite `max_texture_dimension_2d`
/// garantie par tout backend.
pub const MAX_SHADOW_RESOLUTION: u32 = 8192;

/// Le volume monde couvert par les ombres directionnelles : une boîte
/// orthographique ALIGNÉE SUR LA LUMIÈRE. `half_extents.x`/`.y` sont les
/// demi-étendues latérales (perpendiculaires aux rayons), `half_extents.z`
/// la demi-profondeur LE LONG des rayons. Le volume est EXPLICITE et
/// indépendant de la caméra : les ombres sont stables par construction
/// pendant ses mouvements. Le fitting caméra (et les cascades) sont les
/// extensions notées.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowVolume {
    /// Le centre monde du volume.
    pub center: Vec3,
    /// Les demi-étendues du volume dans le repère de la lumière
    /// (x, y latéraux ; z le long des rayons) — strictement positives.
    pub half_extents: Vec3,
}

impl ShadowVolume {
    /// Un volume centré, aux demi-étendues données.
    pub fn new(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            center,
            half_extents,
        }
    }
}

/// Description des ombres de la lumière directionnelle principale — un
/// réglage PERSISTANT du Renderer (le patron de l'environnement), que
/// `clear_draws` ne touche pas. La lumière qui projette est la PREMIÈRE
/// directionnelle activée et valide de la frame ; sans directionnelle,
/// la passe d'ombre est simplement absente — jamais une erreur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalShadowDescriptor {
    /// La résolution (carrée) de la shadow map, en texels —
    /// [`MIN_SHADOW_RESOLUTION`]..=[`MAX_SHADOW_RESOLUTION`].
    pub resolution: u32,
    /// Le volume monde couvert par les ombres.
    pub volume: ShadowVolume,
    /// Le biais de profondeur, en unités de profondeur light-clip (0..1
    /// sur `2 × half_extents.z`) — soustrait à la profondeur comparée,
    /// contre l'acné d'ombre. Réglable à chaud, sans recréation.
    pub depth_bias: f32,
    /// Le biais de normale, en unités MONDE — le point échantillonné est
    /// écarté le long de la normale de la surface, contre l'acné des
    /// surfaces rasantes. Réglable à chaud, sans recréation.
    pub normal_bias: f32,
}

impl DirectionalShadowDescriptor {
    /// Descripteur aux défauts du moteur : 2048 texels, biais doux.
    pub fn new(volume: ShadowVolume) -> Self {
        Self {
            resolution: 2048,
            volume,
            depth_bias: 0.002,
            normal_bias: 0.02,
        }
    }

    /// Règle la résolution de la shadow map.
    pub fn with_resolution(mut self, resolution: u32) -> Self {
        self.resolution = resolution;
        self
    }

    /// Règle le biais de profondeur (unités light-clip).
    pub fn with_depth_bias(mut self, depth_bias: f32) -> Self {
        self.depth_bias = depth_bias;
        self
    }

    /// Règle le biais de normale (unités monde).
    pub fn with_normal_bias(mut self, normal_bias: f32) -> Self {
        self.normal_bias = normal_bias;
        self
    }

    /// Les règles de cohérence du descripteur — l'autorité, appliquée
    /// AVANT tout appel backend : résolution bornée, volume fini aux
    /// étendues strictement positives, biais finis et positifs ou nuls.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.resolution < MIN_SHADOW_RESOLUTION || self.resolution > MAX_SHADOW_RESOLUTION {
            return Err(ChaosError::Graphics(format!(
                "directional shadow: resolution must be within \
                 {MIN_SHADOW_RESOLUTION}..={MAX_SHADOW_RESOLUTION}, got {}",
                self.resolution
            )));
        }
        if !self.volume.center.is_finite() {
            return Err(ChaosError::Graphics(String::from(
                "directional shadow: the volume center is non-finite",
            )));
        }
        let extents = self.volume.half_extents;
        if !extents.is_finite() || extents.x <= 0.0 || extents.y <= 0.0 || extents.z <= 0.0 {
            return Err(ChaosError::Graphics(format!(
                "directional shadow: the volume half extents must be finite and strictly \
                 positive, got ({}, {}, {})",
                extents.x, extents.y, extents.z
            )));
        }
        if !self.depth_bias.is_finite() || self.depth_bias < 0.0 {
            return Err(ChaosError::Graphics(format!(
                "directional shadow: depth bias must be finite and non-negative, got {}",
                self.depth_bias
            )));
        }
        if !self.normal_bias.is_finite() || self.normal_bias < 0.0 {
            return Err(ChaosError::Graphics(format!(
                "directional shadow: normal bias must be finite and non-negative, got {}",
                self.normal_bias
            )));
        }
        Ok(())
    }
}

/// L'inspection des ombres configurées — le miroir lisible de l'état,
/// pour les outils (éditeur, débogage).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalShadowInfo {
    /// La résolution (carrée) de la shadow map.
    pub resolution: u32,
    /// Le volume monde couvert.
    pub volume: ShadowVolume,
    /// Le biais de profondeur (unités light-clip).
    pub depth_bias: f32,
    /// Le biais de normale (unités monde).
    pub normal_bias: f32,
}

/// Ce que le BACKEND doit savoir pour matérialiser les ombres : la
/// ressource, pas la politique — l'audience implémenteur de backend.
/// Volume, biais et lumière voyagent par le plan de frame, à chaque frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowConfig {
    /// La résolution (carrée) de la shadow map à posséder.
    pub resolution: u32,
}

/// La vue-projection de la LUMIÈRE : une orthographique 0..1 alignée sur
/// la direction des rayons, cadrée sur le volume. L'œil recule d'une
/// demi-profondeur depuis le centre — le volume est couvert exactement.
/// Un up de secours prend le relais quand la direction est quasi
/// verticale (le cas du soleil au zénith), jamais une matrice dégénérée.
pub fn light_view_projection(direction: Vec3, volume: &ShadowVolume) -> Mat4 {
    let direction = direction.normalize();
    let up = if direction.dot(math::world::UP).abs() > 0.99 {
        Vec3::Z
    } else {
        math::world::UP
    };
    let eye = volume.center - direction * volume.half_extents.z;
    let view = math::view::look_to(eye, direction, up);
    let projection = math::projection::orthographic(
        -volume.half_extents.x,
        volume.half_extents.x,
        -volume.half_extents.y,
        volume.half_extents.y,
        0.0,
        2.0 * volume.half_extents.z,
    );
    projection * view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> ShadowVolume {
        ShadowVolume::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(10.0, 8.0, 20.0))
    }

    #[test]
    fn the_descriptor_defaults_are_valid_and_overridable() {
        let descriptor = DirectionalShadowDescriptor::new(volume());
        assert_eq!(descriptor.resolution, 2048);
        assert!(descriptor.validate().is_ok());
        let tuned = descriptor
            .with_resolution(1024)
            .with_depth_bias(0.01)
            .with_normal_bias(0.1);
        assert_eq!(tuned.resolution, 1024);
        assert_eq!(tuned.depth_bias, 0.01);
        assert_eq!(tuned.normal_bias, 0.1);
        assert!(tuned.validate().is_ok());
    }

    #[test]
    fn the_resolution_bounds_are_named() {
        let too_small = DirectionalShadowDescriptor::new(volume()).with_resolution(15);
        let error = too_small.validate().unwrap_err();
        assert!(error.to_string().contains("16..=8192"));
        let too_large = DirectionalShadowDescriptor::new(volume()).with_resolution(8193);
        assert!(too_large.validate().is_err());
        let floor = DirectionalShadowDescriptor::new(volume()).with_resolution(16);
        assert!(floor.validate().is_ok());
        let ceiling = DirectionalShadowDescriptor::new(volume()).with_resolution(8192);
        assert!(ceiling.validate().is_ok());
    }

    #[test]
    fn degenerate_volumes_and_biases_are_named() {
        let flat = DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 10.0),
        ));
        assert!(
            flat.validate()
                .unwrap_err()
                .to_string()
                .contains("half extents")
        );
        let wandering = DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::new(f32::NAN, 0.0, 0.0),
            Vec3::ONE,
        ));
        assert!(
            wandering
                .validate()
                .unwrap_err()
                .to_string()
                .contains("center")
        );
        let biased = DirectionalShadowDescriptor::new(volume()).with_depth_bias(-0.1);
        assert!(
            biased
                .validate()
                .unwrap_err()
                .to_string()
                .contains("depth bias")
        );
        let skewed = DirectionalShadowDescriptor::new(volume()).with_normal_bias(f32::NAN);
        assert!(
            skewed
                .validate()
                .unwrap_err()
                .to_string()
                .contains("normal bias")
        );
    }

    #[test]
    fn the_light_projection_centers_the_volume() {
        let volume = volume();
        let matrix = light_view_projection(Vec3::new(-1.0, -1.0, -1.0), &volume);
        let center = matrix.project_point3(volume.center);
        assert!(center.abs().x < 1e-4);
        assert!(center.abs().y < 1e-4);
        assert!((center.z - 0.5).abs() < 1e-4);
        let behind = matrix.project_point3(volume.center + Vec3::new(100.0, 100.0, 100.0));
        assert!(behind.z < 0.0 || behind.z > 1.0 || behind.x.abs() > 1.0 || behind.y.abs() > 1.0);
    }

    #[test]
    fn a_vertical_light_survives_with_a_fallback_up() {
        let volume = volume();
        let noon = light_view_projection(Vec3::NEG_Y, &volume);
        assert!(noon.is_finite());
        let center = noon.project_point3(volume.center);
        assert!((center.z - 0.5).abs() < 1e-4);
        let skyward = light_view_projection(Vec3::Y, &volume);
        assert!(skyward.is_finite());
    }

    #[test]
    fn an_unnormalized_direction_is_normalized_at_the_gate() {
        let volume = volume();
        let stretched = light_view_projection(Vec3::new(0.0, -5.0, 0.0), &volume);
        let unit = light_view_projection(Vec3::NEG_Y, &volume);
        assert!(stretched.abs_diff_eq(unit, 1e-5));
    }
}
