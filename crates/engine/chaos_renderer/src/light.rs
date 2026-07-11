use chaos_core::Color;
use chaos_core::math::Vec3;

/// La capacité d'éclairage d'une frame en V1 : au-delà, les lumières
/// EXCÉDENTAIRES sont écartées en ordre de soumission (les premières
/// gagnent — prévisible), avec un avertissement par épisode.
pub const MAX_LIGHTS: usize = 16;

/// Une source de lumière — une DONNÉE moteur pure, soumise au renderer à
/// chaque frame (`Renderer::submit_light`), jamais un objet GPU. Les
/// couleurs sont linéaires ; l'intensité est un multiplicateur ; les
/// directions n'ont pas besoin d'être normalisées (le renderer les
/// normalise à la collection). Une lumière désactivée peut rester
/// soumise : elle est simplement écartée de la frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Light {
    /// Une lumière directionnelle (le soleil) : tous les rayons suivent
    /// `direction`, sans position ni atténuation.
    Directional {
        /// La direction DES rayons (du ciel vers la scène).
        direction: Vec3,
        /// La couleur linéaire.
        color: Color,
        /// L'intensité (multiplicateur du diffus).
        intensity: f32,
        /// La lumière participe-t-elle à la frame ?
        enabled: bool,
    },
    /// Une lumière ponctuelle : rayonne depuis `position`, atténuée
    /// jusqu'à `range` (zéro au-delà).
    Point {
        /// La position monde de la source.
        position: Vec3,
        /// La couleur linéaire.
        color: Color,
        /// L'intensité (multiplicateur du diffus).
        intensity: f32,
        /// La portée au-delà de laquelle la contribution est nulle.
        range: f32,
        /// La lumière participe-t-elle à la frame ?
        enabled: bool,
    },
    /// Un spot : une ponctuelle contrainte dans un cône orienté —
    /// pleine intensité sous `inner_angle`, fondu jusqu'à `outer_angle`.
    Spot {
        /// La position monde de la source.
        position: Vec3,
        /// L'axe du cône (le sens des rayons).
        direction: Vec3,
        /// La couleur linéaire.
        color: Color,
        /// L'intensité (multiplicateur du diffus).
        intensity: f32,
        /// La portée au-delà de laquelle la contribution est nulle.
        range: f32,
        /// Le demi-angle (radians) de pleine intensité.
        inner_angle: f32,
        /// Le demi-angle (radians) d'extinction — STRICTEMENT supérieur
        /// à `inner_angle`.
        outer_angle: f32,
        /// La lumière participe-t-elle à la frame ?
        enabled: bool,
    },
}

impl Light {
    /// Une directionnelle activée.
    pub fn directional(direction: Vec3, color: Color, intensity: f32) -> Self {
        Self::Directional {
            direction,
            color,
            intensity,
            enabled: true,
        }
    }

    /// Une ponctuelle activée.
    pub fn point(position: Vec3, color: Color, intensity: f32, range: f32) -> Self {
        Self::Point {
            position,
            color,
            intensity,
            range,
            enabled: true,
        }
    }

    /// Un spot activé.
    pub fn spot(
        position: Vec3,
        direction: Vec3,
        color: Color,
        intensity: f32,
        range: f32,
        inner_angle: f32,
        outer_angle: f32,
    ) -> Self {
        Self::Spot {
            position,
            direction,
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
            enabled: true,
        }
    }

    /// Active ou désactive la lumière — une lumière désactivée reste
    /// soumissible, elle est écartée de la collection de frame.
    pub fn set_enabled(&mut self, value: bool) {
        match self {
            Self::Directional { enabled, .. }
            | Self::Point { enabled, .. }
            | Self::Spot { enabled, .. } => *enabled = value,
        }
    }

    /// La lumière participe-t-elle à la frame ?
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Directional { enabled, .. }
            | Self::Point { enabled, .. }
            | Self::Spot { enabled, .. } => *enabled,
        }
    }

    /// La raison pour laquelle la lumière est invalide, s'il y en a une —
    /// une lumière invalide soumise est écartée avec un avertissement,
    /// jamais envoyée au GPU (NaN, cône dégénéré et compagnie).
    pub(crate) fn invalid_reason(&self) -> Option<&'static str> {
        let finite_color =
            |color: &Color| color.r.is_finite() && color.g.is_finite() && color.b.is_finite();
        match self {
            Self::Directional {
                direction,
                color,
                intensity,
                ..
            } => {
                if !direction.is_finite() || direction.length_squared() < f32::EPSILON {
                    return Some("its direction is zero or non-finite");
                }
                if !intensity.is_finite() || *intensity < 0.0 {
                    return Some("its intensity is negative or non-finite");
                }
                if !finite_color(color) {
                    return Some("its color is non-finite");
                }
                None
            }
            Self::Point {
                position,
                color,
                intensity,
                range,
                ..
            } => {
                if !position.is_finite() {
                    return Some("its position is non-finite");
                }
                if !range.is_finite() || *range <= 0.0 {
                    return Some("its range must be positive and finite");
                }
                if !intensity.is_finite() || *intensity < 0.0 {
                    return Some("its intensity is negative or non-finite");
                }
                if !finite_color(color) {
                    return Some("its color is non-finite");
                }
                None
            }
            Self::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                inner_angle,
                outer_angle,
                ..
            } => {
                if !position.is_finite() {
                    return Some("its position is non-finite");
                }
                if !direction.is_finite() || direction.length_squared() < f32::EPSILON {
                    return Some("its direction is zero or non-finite");
                }
                if !range.is_finite() || *range <= 0.0 {
                    return Some("its range must be positive and finite");
                }
                if !intensity.is_finite() || *intensity < 0.0 {
                    return Some("its intensity is negative or non-finite");
                }
                if !finite_color(color) {
                    return Some("its color is non-finite");
                }
                if !inner_angle.is_finite()
                    || !outer_angle.is_finite()
                    || *inner_angle < 0.0
                    || *outer_angle <= *inner_angle
                {
                    return Some(
                        "its cone is degenerate (outer_angle must be strictly greater than inner_angle)",
                    );
                }
                None
            }
        }
    }

    /// Une copie aux directions NORMALISÉES — la forme envoyée au backend.
    pub(crate) fn normalized(&self) -> Self {
        let mut light = self.clone();
        match &mut light {
            Self::Directional { direction, .. } | Self::Spot { direction, .. } => {
                *direction = direction.normalize();
            }
            Self::Point { .. } => {}
        }
        light
    }
}

/// L'éclairage COLLECTÉ d'une frame — la vue structurée que le backend
/// reçoit dans le plan : l'ambiante et les lumières déjà filtrées
/// (activées, valides), normalisées et tronquées à [`MAX_LIGHTS`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameLights {
    /// La couleur de la lumière ambiante (linéaire).
    pub ambient_color: Color,
    /// L'intensité ambiante — 0 (le défaut) = pas d'ambiante : une
    /// surface éclairée sans aucune lumière est noire.
    pub ambient_intensity: f32,
    /// Les lumières de la frame, en ordre de soumission.
    pub lights: Vec<Light>,
}

impl FrameLights {
    /// La frame porte-t-elle un éclairage non trivial ?
    pub fn is_lit(&self) -> bool {
        !self.lights.is_empty() || self.ambient_intensity > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_enable_and_toggling_works() {
        let mut sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0);
        assert!(sun.is_enabled());
        sun.set_enabled(false);
        assert!(!sun.is_enabled());
        assert!(sun.invalid_reason().is_none());
    }

    #[test]
    fn degenerate_lights_are_named() {
        let zero_direction = Light::directional(Vec3::ZERO, Color::WHITE, 1.0);
        assert!(
            zero_direction
                .invalid_reason()
                .is_some_and(|reason| reason.contains("direction"))
        );
        let negative = Light::point(Vec3::ZERO, Color::WHITE, -1.0, 5.0);
        assert!(
            negative
                .invalid_reason()
                .is_some_and(|reason| reason.contains("intensity"))
        );
        let flat_cone = Light::spot(Vec3::ZERO, Vec3::NEG_Y, Color::WHITE, 1.0, 5.0, 0.4, 0.4);
        assert!(
            flat_cone
                .invalid_reason()
                .is_some_and(|reason| reason.contains("cone"))
        );
        let no_range = Light::point(Vec3::ZERO, Color::WHITE, 1.0, 0.0);
        assert!(
            no_range
                .invalid_reason()
                .is_some_and(|reason| reason.contains("range"))
        );
    }

    #[test]
    fn normalization_only_touches_directions() {
        let sun = Light::directional(Vec3::new(0.0, -2.0, 0.0), Color::WHITE, 1.0);
        let Light::Directional { direction, .. } = sun.normalized() else {
            panic!("kind changed");
        };
        assert!((direction.length() - 1.0).abs() < 1e-6);
        let lamp = Light::point(Vec3::new(0.0, 3.0, 0.0), Color::WHITE, 1.0, 5.0);
        assert_eq!(lamp.normalized(), lamp);
    }
}
