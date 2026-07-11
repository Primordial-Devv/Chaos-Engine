//! Le DEBUG RENDERING du renderer : le langage visuel commun des
//! données spatiales — lignes, rayons, flèches, points, marqueurs,
//! axes, grilles, boîtes, sphères, frustums, lumières. Chaque primitive
//! porte sa couleur, sa durée de vie, sa catégorie, son mode de
//! profondeur et sa passe cible ; la tessellation (pure, testée sans
//! GPU) transforme TOUT en segments de lignes monde, injectés APRÈS les
//! transparents de leur passe. C'est un service du RENDERER — jamais
//! une dépendance vers l'UI, l'éditeur, la physique, l'ECS ou les
//! scènes : les futurs systèmes SOUMETTENT, le renderer dessine.

use std::f32::consts::TAU;

use chaos_core::math::{Aabb, Mat4, Vec3};
use chaos_core::{ChaosError, ChaosResult, Color};

use crate::light::Light;
use crate::pass::PassHandle;
use crate::resources::DebugVertex;

/// La catégorie par défaut des primitives de debug.
pub const DEFAULT_DEBUG_CATEGORY: &str = "general";

/// Le nombre de segments d'un cercle de debug (sphères, cônes de spot).
const CIRCLE_SEGMENTS: u32 = 24;

/// L'inspection du store de debug (`Renderer::debug_stats`) — les
/// comptes de primitives, PAS leur rendu (les toggles n'y paraissent
/// pas : une catégorie désactivée garde ses retenues qui expirent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugStats {
    /// Les primitives de la frame de simulation courante (durée 0).
    pub frame: usize,
    /// Les primitives RETENUES (durée > 0), pas encore expirées.
    pub retained: usize,
}

/// La géométrie d'une primitive de debug — le QUOI, en coordonnées
/// MONDE. La tessellation interne transforme chaque forme en segments
/// de lignes (1 px — la limite V1 portable).
#[derive(Debug, Clone, PartialEq)]
pub enum DebugShape {
    /// Un segment de `from` à `to`.
    Line {
        /// Le départ du segment.
        from: Vec3,
        /// L'arrivée du segment.
        to: Vec3,
    },
    /// Un rayon : origine + direction — la LONGUEUR de `direction` est
    /// la portée dessinée, une petite croix marque l'extrémité.
    Ray {
        /// L'origine du rayon.
        origin: Vec3,
        /// La direction ET la portée (non normalisée).
        direction: Vec3,
    },
    /// Une flèche de `from` à `to` — le fût + quatre segments de tête.
    Arrow {
        /// Le départ (la queue).
        from: Vec3,
        /// L'arrivée (la pointe).
        to: Vec3,
    },
    /// Un point : une croix à trois axes de demi-taille `size`.
    Point {
        /// La position monde.
        position: Vec3,
        /// La demi-taille de la croix.
        size: f32,
    },
    /// Un marqueur : un octaèdre filaire de demi-taille `size` —
    /// distinct du point à l'œil.
    Marker {
        /// La position monde.
        position: Vec3,
        /// La demi-taille de l'octaèdre.
        size: f32,
    },
    /// Les axes d'un repère : X rouge, Y vert, Z bleu, longueur `size` —
    /// la couleur du draw est IGNORÉE (les couleurs canoniques priment).
    Axes {
        /// Le repère dessiné (translation + rotation + échelle).
        transform: Mat4,
        /// La longueur de chaque axe.
        size: f32,
    },
    /// Une grille sur le plan XZ, centrée sur `center` : des lignes tous
    /// les `spacing`, jusqu'à `half_extent` dans les deux directions.
    Grid {
        /// Le centre de la grille (le plan vit à sa hauteur `y`).
        center: Vec3,
        /// La demi-étendue de la grille sur X et Z.
        half_extent: f32,
        /// Le pas entre deux lignes.
        spacing: f32,
    },
    /// Une boîte englobante alignée aux axes — les 12 arêtes.
    Aabb {
        /// La boîte monde.
        bounds: Aabb,
    },
    /// Une sphère filaire : trois grands cercles (plans XY, XZ, YZ).
    Sphere {
        /// Le centre monde.
        center: Vec3,
        /// Le rayon.
        radius: f32,
    },
    /// Un frustum : les 12 arêtes des 8 coins DÉ-PROJETÉS de la matrice
    /// vue-projection (convention de profondeur 0..1 — la nôtre).
    Frustum {
        /// La matrice vue-projection de la vue dessinée (inversible).
        view_projection: Mat4,
    },
    /// La représentation simple d'une lumière : des flèches pour une
    /// directionnelle (depuis `anchor` — elle n'a pas de position), la
    /// sphère de portée pour une ponctuelle, le cône pour un spot.
    Light {
        /// La lumière dessinée (une copie de la donnée).
        light: Light,
        /// L'ancre de dessin d'une DIRECTIONNELLE (ignorée sinon).
        anchor: Vec3,
    },
}

/// Le comportement de PROFONDEUR d'une primitive de debug. Dans les
/// deux modes la profondeur n'est JAMAIS écrite — le debug n'occulte
/// pas la scène.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DebugDepth {
    /// Testé contre la profondeur de la scène : la primitive est
    /// occludée par ce qui passe devant elle. Le défaut.
    #[default]
    Scene,
    /// Ignore la profondeur : dessiné PAR-DESSUS tout — les repères qui
    /// doivent rester visibles à travers la scène.
    Overlay,
}

/// Une primitive de debug complète — la forme et ses réglages. Se
/// construit par les builders (`DebugDraw::line(a, b).with_color(…)`)
/// et se soumet via `Renderer::queue_debug`.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugDraw {
    /// La géométrie, en coordonnées monde.
    pub shape: DebugShape,
    /// La couleur RGBA linéaire (l'alpha est respecté — les pipelines
    /// debug mélangent). Ignorée par `Axes` (couleurs canoniques).
    pub color: Color,
    /// La durée de vie en secondes : `0.0` (défaut) = la frame de
    /// SIMULATION courante (vidée par `clear_draws`, comme les draws) ;
    /// `> 0` = RETENUE — survit à `clear_draws`, décomptée par
    /// `Renderer::advance_debug_time`, expire automatiquement.
    pub duration: f32,
    /// La catégorie — le levier d'activation par famille
    /// (`Renderer::set_debug_category_enabled`).
    pub category: String,
    /// Le comportement de profondeur (testé ou par-dessus tout).
    pub depth: DebugDepth,
    /// La passe cible — `None` (défaut) = la passe principale.
    pub pass: Option<PassHandle>,
}

impl DebugDraw {
    fn new(shape: DebugShape) -> Self {
        Self {
            shape,
            color: Color::WHITE,
            duration: 0.0,
            category: String::from(DEFAULT_DEBUG_CATEGORY),
            depth: DebugDepth::Scene,
            pass: None,
        }
    }

    /// Un segment de `from` à `to`.
    pub fn line(from: Vec3, to: Vec3) -> Self {
        Self::new(DebugShape::Line { from, to })
    }

    /// Un rayon depuis `origin` — la longueur de `direction` est la
    /// portée dessinée.
    pub fn ray(origin: Vec3, direction: Vec3) -> Self {
        Self::new(DebugShape::Ray { origin, direction })
    }

    /// Une flèche de `from` vers `to`.
    pub fn arrow(from: Vec3, to: Vec3) -> Self {
        Self::new(DebugShape::Arrow { from, to })
    }

    /// Un point (croix à trois axes) de demi-taille `size`.
    pub fn point(position: Vec3, size: f32) -> Self {
        Self::new(DebugShape::Point { position, size })
    }

    /// Un marqueur (octaèdre filaire) de demi-taille `size`.
    pub fn marker(position: Vec3, size: f32) -> Self {
        Self::new(DebugShape::Marker { position, size })
    }

    /// Les axes d'un repère (X rouge, Y vert, Z bleu), longueur `size`.
    pub fn axes(transform: Mat4, size: f32) -> Self {
        Self::new(DebugShape::Axes { transform, size })
    }

    /// Une grille sur le plan XZ centrée sur `center`.
    pub fn grid(center: Vec3, half_extent: f32, spacing: f32) -> Self {
        Self::new(DebugShape::Grid {
            center,
            half_extent,
            spacing,
        })
    }

    /// Les 12 arêtes d'une boîte englobante monde.
    pub fn aabb(bounds: Aabb) -> Self {
        Self::new(DebugShape::Aabb { bounds })
    }

    /// Une sphère filaire (trois grands cercles).
    pub fn sphere(center: Vec3, radius: f32) -> Self {
        Self::new(DebugShape::Sphere { center, radius })
    }

    /// Un frustum depuis la matrice vue-projection de sa vue.
    pub fn frustum(view_projection: Mat4) -> Self {
        Self::new(DebugShape::Frustum { view_projection })
    }

    /// La représentation d'une lumière — la couleur du draw est
    /// initialisée à CELLE de la lumière (remplaçable par `with_color`).
    /// `anchor` est le point de dessin d'une directionnelle (sans
    /// position propre) ; les ponctuelles et les spots l'ignorent.
    pub fn light(light: &Light, anchor: Vec3) -> Self {
        let color = match light {
            Light::Directional { color, .. }
            | Light::Point { color, .. }
            | Light::Spot { color, .. } => *color,
        };
        let mut draw = Self::new(DebugShape::Light {
            light: light.clone(),
            anchor,
        });
        draw.color = color;
        draw
    }

    /// Remplace la couleur.
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Fixe la durée de vie en secondes (`> 0` = retenue et expirée
    /// automatiquement).
    pub fn with_duration(mut self, duration: f32) -> Self {
        self.duration = duration;
        self
    }

    /// Remplace la catégorie.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Passe la primitive en OVERLAY : dessinée par-dessus tout.
    pub fn overlay(mut self) -> Self {
        self.depth = DebugDepth::Overlay;
        self
    }

    /// Cible une passe déclarée (la principale par défaut).
    pub fn for_pass(mut self, pass: PassHandle) -> Self {
        self.pass = Some(pass);
        self
    }

    /// Vérifie la cohérence PROPRE de la primitive (géométrie finie,
    /// tailles positives, durée saine, frustum inversible, catégorie
    /// non vide) — l'existence de la passe cible appartient au
    /// Renderer. Une primitive invalide est ÉCARTÉE au submit avec un
    /// warn, jamais stockée.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.category.is_empty() {
            return Err(refused("category cannot be empty"));
        }
        if !self.duration.is_finite() || self.duration < 0.0 {
            return Err(refused("duration must be finite and non-negative"));
        }
        let color = [self.color.r, self.color.g, self.color.b, self.color.a];
        if !color.iter().all(|channel| channel.is_finite()) {
            return Err(refused("color must be finite"));
        }
        match &self.shape {
            DebugShape::Line { from, to } | DebugShape::Arrow { from, to } => {
                finite_points(&[*from, *to])
            }
            DebugShape::Ray { origin, direction } => finite_points(&[*origin, *direction]),
            DebugShape::Point { position, size } | DebugShape::Marker { position, size } => {
                finite_points(&[*position])?;
                positive_size(*size, "size")
            }
            DebugShape::Axes { transform, size } => {
                if !transform.is_finite() {
                    return Err(refused("axes transform must be finite"));
                }
                positive_size(*size, "size")
            }
            DebugShape::Grid {
                center,
                half_extent,
                spacing,
            } => {
                finite_points(&[*center])?;
                positive_size(*half_extent, "half extent")?;
                positive_size(*spacing, "spacing")
            }
            DebugShape::Aabb { bounds } => finite_points(&[bounds.min, bounds.max]),
            DebugShape::Sphere { center, radius } => {
                finite_points(&[*center])?;
                positive_size(*radius, "radius")
            }
            DebugShape::Frustum { view_projection } => {
                let determinant = view_projection.determinant();
                if !view_projection.is_finite() || !determinant.is_finite() || determinant == 0.0 {
                    return Err(refused("frustum view-projection is not invertible"));
                }
                Ok(())
            }
            DebugShape::Light { light, anchor } => {
                finite_points(&[*anchor])?;
                let finite = match light {
                    Light::Directional { direction, .. } => direction.is_finite(),
                    Light::Point {
                        position, range, ..
                    } => position.is_finite() && range.is_finite(),
                    Light::Spot {
                        position,
                        direction,
                        range,
                        inner_angle,
                        outer_angle,
                        ..
                    } => {
                        position.is_finite()
                            && direction.is_finite()
                            && range.is_finite()
                            && inner_angle.is_finite()
                            && outer_angle.is_finite()
                    }
                };
                if !finite {
                    return Err(refused("light carries non-finite values"));
                }
                Ok(())
            }
        }
    }
}

fn refused(reason: &str) -> ChaosError {
    ChaosError::Graphics(format!("debug draw refused: {reason}"))
}

fn finite_points(points: &[Vec3]) -> ChaosResult<()> {
    if points.iter().all(|point| point.is_finite()) {
        Ok(())
    } else {
        Err(refused("geometry carries non-finite values"))
    }
}

fn positive_size(value: f32, name: &str) -> ChaosResult<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(refused(&format!("{name} must be finite and positive")))
    }
}

impl DebugShape {
    /// Tessellate la forme en SEGMENTS (paires de sommets, topologie
    /// lignes) ajoutés à `out` — pur, en espace monde, la couleur portée
    /// par sommet. C'est l'AUTORITÉ géométrique du debug : le renderer
    /// l'appelle au resolve, les outils et les tests peuvent la
    /// consommer telle quelle. Ne suppose que des données VALIDÉES
    /// (`DebugDraw::validate`).
    pub fn tessellate(&self, color: Color, out: &mut Vec<DebugVertex>) {
        tessellate(self, color, out);
    }
}

fn tessellate(shape: &DebugShape, color: Color, out: &mut Vec<DebugVertex>) {
    let rgba = [color.r, color.g, color.b, color.a];
    match shape {
        DebugShape::Line { from, to } => segment(out, *from, *to, rgba),
        DebugShape::Ray { origin, direction } => {
            let end = *origin + *direction;
            segment(out, *origin, end, rgba);
            cross(out, end, direction.length() * 0.05, rgba);
        }
        DebugShape::Arrow { from, to } => arrow(out, *from, *to, rgba),
        DebugShape::Point { position, size } => cross(out, *position, *size, rgba),
        DebugShape::Marker { position, size } => octahedron(out, *position, *size, rgba),
        DebugShape::Axes { transform, size } => {
            let origin = transform.w_axis.truncate();
            for (axis, canonical) in [
                (Vec3::X, [1.0, 0.0, 0.0, 1.0]),
                (Vec3::Y, [0.0, 1.0, 0.0, 1.0]),
                (Vec3::Z, [0.0, 0.0, 1.0, 1.0]),
            ] {
                let end = origin + transform.transform_vector3(axis) * *size;
                segment(out, origin, end, canonical);
            }
        }
        DebugShape::Grid {
            center,
            half_extent,
            spacing,
        } => {
            let steps = (half_extent / spacing).floor() as i32;
            for index in -steps..=steps {
                let offset = index as f32 * spacing;
                segment(
                    out,
                    Vec3::new(center.x + offset, center.y, center.z - half_extent),
                    Vec3::new(center.x + offset, center.y, center.z + half_extent),
                    rgba,
                );
                segment(
                    out,
                    Vec3::new(center.x - half_extent, center.y, center.z + offset),
                    Vec3::new(center.x + half_extent, center.y, center.z + offset),
                    rgba,
                );
            }
        }
        DebugShape::Aabb { bounds } => box_edges(
            out,
            &corners_of(|x, y, z| {
                Vec3::new(
                    if x { bounds.max.x } else { bounds.min.x },
                    if y { bounds.max.y } else { bounds.min.y },
                    if z { bounds.max.z } else { bounds.min.z },
                )
            }),
            rgba,
        ),
        DebugShape::Sphere { center, radius } => {
            circle(out, *center, Vec3::X, Vec3::Y, *radius, rgba);
            circle(out, *center, Vec3::X, Vec3::Z, *radius, rgba);
            circle(out, *center, Vec3::Y, Vec3::Z, *radius, rgba);
        }
        DebugShape::Frustum { view_projection } => {
            let inverse = view_projection.inverse();
            // Les 8 coins dé-projetés du cube clip (z 0..1 — nos
            // conventions), dans l'ordre du treillis (x, y, near/far).
            box_edges(
                out,
                &corners_of(|x, y, z| {
                    inverse.project_point3(Vec3::new(
                        if x { 1.0 } else { -1.0 },
                        if y { 1.0 } else { -1.0 },
                        if z { 1.0 } else { 0.0 },
                    ))
                }),
                rgba,
            );
        }
        DebugShape::Light { light, anchor } => light_shape(out, light, *anchor, rgba),
    }
}

fn segment(out: &mut Vec<DebugVertex>, from: Vec3, to: Vec3, color: [f32; 4]) {
    out.push(DebugVertex {
        position: from.to_array(),
        color,
    });
    out.push(DebugVertex {
        position: to.to_array(),
        color,
    });
}

/// Une croix à trois axes — le point et les extrémités de rayon.
fn cross(out: &mut Vec<DebugVertex>, center: Vec3, size: f32, color: [f32; 4]) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        segment(out, center - axis * size, center + axis * size, color);
    }
}

/// Le fût + quatre segments de tête, dans les deux plans orthogonaux à
/// la flèche.
fn arrow(out: &mut Vec<DebugVertex>, from: Vec3, to: Vec3, color: [f32; 4]) {
    segment(out, from, to, color);
    let shaft = to - from;
    let Some(direction) = shaft.try_normalize() else {
        return;
    };
    let head = shaft.length() * 0.2;
    let side = direction.any_orthonormal_vector();
    let other = direction.cross(side);
    let base = to - direction * head;
    for wing in [side, -side, other, -other] {
        segment(out, to, base + wing * head * 0.5, color);
    }
}

/// L'octaèdre filaire du marqueur : 6 sommets (±size par axe), 12 arêtes.
fn octahedron(out: &mut Vec<DebugVertex>, center: Vec3, size: f32, color: [f32; 4]) {
    let top = center + Vec3::Y * size;
    let bottom = center - Vec3::Y * size;
    let ring = [
        center + Vec3::X * size,
        center + Vec3::Z * size,
        center - Vec3::X * size,
        center - Vec3::Z * size,
    ];
    for index in 0..4 {
        let next = ring[(index + 1) % 4];
        segment(out, ring[index], next, color);
        segment(out, top, ring[index], color);
        segment(out, bottom, ring[index], color);
    }
}

/// Un cercle dans le plan porté par (`u`, `v`).
fn circle(
    out: &mut Vec<DebugVertex>,
    center: Vec3,
    u: Vec3,
    v: Vec3,
    radius: f32,
    color: [f32; 4],
) {
    let point = |index: u32| {
        let angle = index as f32 / CIRCLE_SEGMENTS as f32 * TAU;
        center + (u * angle.cos() + v * angle.sin()) * radius
    };
    for index in 0..CIRCLE_SEGMENTS {
        segment(out, point(index), point(index + 1), color);
    }
}

/// Les 8 coins d'une boîte par le treillis (x, y, z) ∈ {faux, vrai}³.
fn corners_of(corner: impl Fn(bool, bool, bool) -> Vec3) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    for (index, slot) in corners.iter_mut().enumerate() {
        *slot = corner(index & 1 != 0, index & 2 != 0, index & 4 != 0);
    }
    corners
}

/// Les 12 arêtes d'une boîte aux coins en treillis (bit 0 = x, bit 1 =
/// y, bit 2 = z) : chaque paire de coins qui ne diffère que d'UN bit.
fn box_edges(out: &mut Vec<DebugVertex>, corners: &[Vec3; 8], color: [f32; 4]) {
    for index in 0..8 {
        for bit in [1usize, 2, 4] {
            let neighbour = index | bit;
            if neighbour != index {
                segment(out, corners[index], corners[neighbour], color);
            }
        }
    }
}

/// La représentation d'une lumière : la DONNÉE dessinée, pas son rendu.
fn light_shape(out: &mut Vec<DebugVertex>, light: &Light, anchor: Vec3, color: [f32; 4]) {
    match light {
        // La directionnelle : trois flèches parallèles depuis l'ancre —
        // les rayons du soleil.
        Light::Directional { direction, .. } => {
            let Some(direction) = direction.try_normalize() else {
                cross(out, anchor, 0.25, color);
                return;
            };
            let side = direction.any_orthonormal_vector();
            for offset in [Vec3::ZERO, side * 0.5, -side * 0.5] {
                let from = anchor + offset;
                arrow(out, from, from + direction * 2.0, color);
            }
        }
        // La ponctuelle : la sphère de sa portée + son centre.
        Light::Point {
            position, range, ..
        } => {
            circle(out, *position, Vec3::X, Vec3::Y, *range, color);
            circle(out, *position, Vec3::X, Vec3::Z, *range, color);
            circle(out, *position, Vec3::Y, Vec3::Z, *range, color);
            cross(out, *position, range * 0.1, color);
        }
        // Le spot : les cercles inner/outer à sa portée + quatre
        // génératrices vers le cercle externe.
        Light::Spot {
            position,
            direction,
            range,
            inner_angle,
            outer_angle,
            ..
        } => {
            let Some(direction) = direction.try_normalize() else {
                cross(out, *position, 0.25, color);
                return;
            };
            let side = direction.any_orthonormal_vector();
            let other = direction.cross(side);
            let far = *position + direction * *range;
            let outer = *range * outer_angle.tan();
            circle(out, far, side, other, outer, color);
            circle(out, far, side, other, *range * inner_angle.tan(), color);
            for wing in [side, -side, other, -other] {
                segment(out, *position, far + wing * outer, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::math::projection;

    use super::*;

    fn vertices(shape: DebugShape) -> Vec<DebugVertex> {
        let mut out = Vec::new();
        tessellate(&shape, Color::WHITE, &mut out);
        out
    }

    #[test]
    fn every_shape_tessellates_to_its_exact_segment_count() {
        // Le contrat de la topologie lignes : des PAIRES, aux comptes
        // verrouillés — un changement de tessellation se voit ici.
        let cases = [
            (
                vertices(DebugShape::Line {
                    from: Vec3::ZERO,
                    to: Vec3::X,
                }),
                2,
            ),
            (
                vertices(DebugShape::Ray {
                    origin: Vec3::ZERO,
                    direction: Vec3::X,
                }),
                8,
            ),
            (
                vertices(DebugShape::Arrow {
                    from: Vec3::ZERO,
                    to: Vec3::X,
                }),
                10,
            ),
            (
                vertices(DebugShape::Point {
                    position: Vec3::ZERO,
                    size: 1.0,
                }),
                6,
            ),
            (
                vertices(DebugShape::Marker {
                    position: Vec3::ZERO,
                    size: 1.0,
                }),
                24,
            ),
            (
                vertices(DebugShape::Axes {
                    transform: Mat4::IDENTITY,
                    size: 1.0,
                }),
                6,
            ),
            (
                vertices(DebugShape::Aabb {
                    bounds: Aabb::from_points([Vec3::ZERO, Vec3::ONE]).unwrap(),
                }),
                24,
            ),
            (
                vertices(DebugShape::Sphere {
                    center: Vec3::ZERO,
                    radius: 1.0,
                }),
                144,
            ),
            (
                vertices(DebugShape::Frustum {
                    view_projection: Mat4::IDENTITY,
                }),
                24,
            ),
        ];
        for (index, (vertices, expected)) in cases.iter().enumerate() {
            assert_eq!(vertices.len(), *expected, "shape #{index}");
            assert_eq!(vertices.len() % 2, 0, "shape #{index} must be segments");
        }
        // La grille : demi-étendue 10, pas 1 → 21 lignes par direction.
        let grid = vertices(DebugShape::Grid {
            center: Vec3::ZERO,
            half_extent: 10.0,
            spacing: 1.0,
        });
        assert_eq!(grid.len(), 2 * (2 * 21));
    }

    #[test]
    fn the_line_carries_its_endpoints_and_color() {
        let mut out = Vec::new();
        tessellate(
            &DebugShape::Line {
                from: Vec3::new(1.0, 2.0, 3.0),
                to: Vec3::new(4.0, 5.0, 6.0),
            },
            Color::rgba(0.5, 0.25, 0.125, 0.75),
            &mut out,
        );
        assert_eq!(out[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(out[1].position, [4.0, 5.0, 6.0]);
        assert_eq!(out[0].color, [0.5, 0.25, 0.125, 0.75]);
        assert_eq!(out[1].color, [0.5, 0.25, 0.125, 0.75]);
    }

    #[test]
    fn the_axes_ignore_the_draw_color_for_the_canonical_rgb() {
        let transform = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let mut out = Vec::new();
        tessellate(
            &DebugShape::Axes {
                transform,
                size: 2.0,
            },
            Color::BLACK,
            &mut out,
        );
        // X rouge part de l'origine du repère et va à +2 sur X.
        assert_eq!(out[0].position, [1.0, 0.0, 0.0]);
        assert_eq!(out[1].position, [3.0, 0.0, 0.0]);
        assert_eq!(out[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(out[3].color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(out[5].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn the_frustum_corners_roundtrip_an_orthographic_box() {
        // L'ortho bénie cadre un volume connu : les coins dé-projetés
        // doivent retomber EXACTEMENT sur les coins du volume.
        let mut out = Vec::new();
        tessellate(
            &DebugShape::Frustum {
                view_projection: projection::orthographic(-2.0, 2.0, -1.0, 1.0, 0.0, 10.0),
            },
            Color::WHITE,
            &mut out,
        );
        let xs: Vec<f32> = out.iter().map(|vertex| vertex.position[0]).collect();
        let zs: Vec<f32> = out.iter().map(|vertex| vertex.position[2]).collect();
        assert!(xs.iter().all(|x| (x.abs() - 2.0).abs() < 1e-4));
        assert!(zs.iter().all(|z| z.abs() < 1e-4 || (z + 10.0).abs() < 1e-3));
    }

    #[test]
    fn the_lights_draw_their_data() {
        // La ponctuelle : trois cercles À SA PORTÉE + la croix centrale.
        let point = vertices(DebugShape::Light {
            light: Light::point(Vec3::new(0.0, 2.0, 0.0), Color::WHITE, 1.0, 3.0),
            anchor: Vec3::ZERO,
        });
        assert_eq!(point.len(), 144 + 6);
        let center = Vec3::new(0.0, 2.0, 0.0);
        assert!(point[..144].iter().all(|vertex| {
            let distance = (Vec3::from_array(vertex.position) - center).length();
            (distance - 3.0).abs() < 1e-3
        }));
        // La directionnelle : trois flèches (30 sommets).
        let sun = vertices(DebugShape::Light {
            light: Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0),
            anchor: Vec3::new(0.0, 5.0, 0.0),
        });
        assert_eq!(sun.len(), 30);
        // Le spot : deux cercles + quatre génératrices.
        let spot = vertices(DebugShape::Light {
            light: Light::spot(Vec3::ZERO, Vec3::NEG_Y, Color::WHITE, 1.0, 4.0, 0.3, 0.5),
            anchor: Vec3::ZERO,
        });
        assert_eq!(spot.len(), 2 * 48 + 8);
    }

    #[test]
    fn the_builders_carry_their_settings() {
        let draw = DebugDraw::line(Vec3::ZERO, Vec3::X)
            .with_color(Color::rgba(1.0, 0.0, 0.0, 0.5))
            .with_duration(2.0)
            .with_category("physics")
            .overlay();
        assert_eq!(draw.color, Color::rgba(1.0, 0.0, 0.0, 0.5));
        assert_eq!(draw.duration, 2.0);
        assert_eq!(draw.category, "physics");
        assert_eq!(draw.depth, DebugDepth::Overlay);
        assert_eq!(draw.pass, None);
        assert_eq!(draw.validate(), Ok(()));
        // Les défauts : catégorie générale, profondeur testée, une frame.
        let plain = DebugDraw::point(Vec3::ZERO, 0.1);
        assert_eq!(plain.category, DEFAULT_DEBUG_CATEGORY);
        assert_eq!(plain.depth, DebugDepth::Scene);
        assert_eq!(plain.duration, 0.0);
        // La lumière prend SA couleur.
        let lamp = DebugDraw::light(
            &Light::point(Vec3::ZERO, Color::rgb(0.0, 1.0, 0.0), 1.0, 2.0),
            Vec3::ZERO,
        );
        assert_eq!(lamp.color, Color::rgb(0.0, 1.0, 0.0));
    }

    #[test]
    fn invalid_draws_are_refused_by_name() {
        let cases = [
            (
                DebugDraw::line(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X),
                "non-finite",
            ),
            (DebugDraw::point(Vec3::ZERO, 0.0), "size"),
            (DebugDraw::sphere(Vec3::ZERO, -1.0), "radius"),
            (DebugDraw::grid(Vec3::ZERO, 10.0, 0.0), "spacing"),
            (
                DebugDraw::frustum(Mat4::from_scale(Vec3::new(1.0, 1.0, 0.0))),
                "not invertible",
            ),
            (
                DebugDraw::line(Vec3::ZERO, Vec3::X).with_duration(-1.0),
                "duration",
            ),
            (
                DebugDraw::line(Vec3::ZERO, Vec3::X).with_category(""),
                "category",
            ),
        ];
        for (draw, needle) in cases {
            let error = draw.validate().unwrap_err().to_string();
            assert!(error.contains(needle), "{error} should name {needle}");
        }
    }
}
