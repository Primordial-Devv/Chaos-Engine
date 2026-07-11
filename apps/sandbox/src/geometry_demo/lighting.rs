//! L'ÉCLAIRAGE de la démo : l'ambiante douce, les OMBRES
//! directionnelles (volume explicite, toggle N), le soleil chaud
//! (toggle K via `enabled`), trois ponctuelles colorées en orbite
//! suivies de leurs marqueurs Unlit — on VOIT où sont les sources — et
//! un spot cyan plongeant sur le cube central. Les lumières sont
//! re-soumises chaque update, comme les draws.

use chaos_engine::{
    ChaosResult, Color, DirectionalShadowDescriptor, DrawCommand, Light, MaterialDescriptor,
    MaterialHandle, MaterialModel, MeshHandle, Renderer, ShadowVolume, TexturedGeometry, Transform,
    light_view_projection,
    math::{Mat4, Vec3},
};

/// Les trois ponctuelles orbitantes de la démo : couleur, rayon d'orbite
/// et vitesse angulaire — les marqueurs visuels suivent les mêmes
/// valeurs. Les rayons sont ÉTAGÉS (2.8 / 3.5 / 4.5) : les flaques de
/// couleur se séparent au lieu de se superposer en une tache blanche au
/// centre, et restent dans la zone centrale — les pavillons des bords
/// (±7) ne sont jamais traversés.
const ORBIT_LIGHTS: [(Color, f32, f32); 3] = [
    (Color::rgb(1.0, 0.25, 0.2), 3.5, 0.6),
    (Color::rgb(0.25, 1.0, 0.3), 2.8, -0.85),
    (Color::rgb(0.3, 0.45, 1.0), 4.5, 0.4),
];

/// La direction du soleil de la démo — partagée entre la lumière
/// soumise et le frustum du volume d'ombre dessiné par le debug (F).
const SUN_DIRECTION: Vec3 = Vec3::new(-0.4, -1.0, -0.3);

/// Les réglages d'ombre de la démo : un volume monde EXPLICITE qui
/// couvre toute la scène (sol 20×20, pavillons compris) — stable par
/// construction, indépendant de la caméra. Partagés entre l'init et le
/// toggle N.
fn demo_shadow_settings() -> DirectionalShadowDescriptor {
    DirectionalShadowDescriptor::new(ShadowVolume::new(
        Vec3::new(0.0, 1.5, 0.0),
        Vec3::new(14.0, 10.0, 14.0),
    ))
}

/// Le rig d'éclairage : les marqueurs des ponctuelles et les deux
/// toggles (soleil, ombres).
pub(super) struct Lighting {
    marker_mesh: MeshHandle,
    marker_materials: Vec<MaterialHandle>,
    sun_disabled: bool,
    shadows_disabled: bool,
}

impl Lighting {
    /// Pose l'ambiante (0.02 — l'IBL de l'environnement prend le
    /// relais), configure les ombres directionnelles (2048 texels sur le
    /// volume explicite) et crée les trois marqueurs orbitaux.
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        renderer.set_ambient_light(Color::WHITE, 0.02);
        renderer.set_directional_shadow(&demo_shadow_settings())?;

        let marker_cube = TexturedGeometry::cube([0.0, 0.0, 0.0], 0.12);
        let marker_mesh = renderer.create_textured_mesh("demo.marker", &marker_cube)?;
        let mut marker_materials = Vec::new();
        for (index, (color, _, _)) in ORBIT_LIGHTS.iter().enumerate() {
            marker_materials.push(
                renderer.create_material(
                    &MaterialDescriptor::new(format!("demo.marker.{index}"), MaterialModel::Unlit)
                        .with_base_color(*color),
                )?,
            );
        }
        Ok(Self {
            marker_mesh,
            marker_materials,
            sun_disabled: false,
            shadows_disabled: false,
        })
    }

    /// L'INSTANTANÉ des lumières de la frame à l'instant `t` — la
    /// source UNIQUE partagée entre la soumission (`frame`) et le
    /// dessin de debug (J) : le soleil (togglé par K via `enabled`),
    /// les trois ponctuelles orbitantes, le spot cyan.
    pub(super) fn lights_snapshot(&self, t: f32) -> Vec<Light> {
        let mut sun = Light::directional(SUN_DIRECTION, Color::rgb(1.0, 0.96, 0.88), 0.9);
        sun.set_enabled(!self.sun_disabled);
        let mut lights = vec![sun];
        for (index, (color, radius, speed)) in ORBIT_LIGHTS.iter().enumerate() {
            let angle = t * speed + f32::from(u8::try_from(index).unwrap_or(0)) * 2.1;
            let position = Vec3::new(angle.cos() * radius, 1.1, angle.sin() * radius);
            lights.push(Light::point(position, *color, 2.5, 5.0));
        }
        lights.push(Light::spot(
            Vec3::new(0.0, 4.5, 0.0),
            Vec3::NEG_Y,
            Color::rgb(0.4, 0.95, 1.0),
            3.0,
            9.0,
            0.25,
            0.45,
        ));
        lights
    }

    /// La vue-projection du VOLUME d'ombre de la démo — le frustum que
    /// le debug dessine (F) : l'ortho de la lumière cadrée sur le
    /// volume explicite, la même autorité que la passe d'ombre.
    pub(super) fn shadow_frustum(&self) -> Mat4 {
        light_view_projection(SUN_DIRECTION, &demo_shadow_settings().volume)
    }

    /// Les lumières de la frame, soumises depuis l'instantané — chaque
    /// ponctuelle orbitante est SUIVIE de son marqueur Unlit.
    pub(super) fn frame(&self, renderer: &mut Renderer, t: f32) {
        let mut point_index = 0;
        for light in self.lights_snapshot(t) {
            if let Light::Point { position, .. } = light {
                if let Some(marker_material) = self.marker_materials.get(point_index).copied() {
                    renderer.queue_draw(DrawCommand {
                        mesh: self.marker_mesh,
                        material: marker_material,
                        transform: Transform::from_translation(position),
                    });
                }
                point_index += 1;
            }
            renderer.submit_light(light);
        }
    }

    /// K : bascule la lumière directionnelle (le « soleil ») — la
    /// soumission continue mais avec `enabled = false`. Les ombres
    /// disparaissent avec lui.
    pub(super) fn toggle_sun(&mut self) {
        self.sun_disabled = !self.sun_disabled;
        log::info!(
            "directional light {}",
            if self.sun_disabled {
                "disabled"
            } else {
                "enabled"
            }
        );
    }

    /// N : bascule les ombres directionnelles (set/clear) — la shadow
    /// map est libérée puis recréée : le chemin de recréation propre des
    /// ressources, exercé en réel.
    pub(super) fn toggle_shadows(&mut self, renderer: &mut Renderer) {
        self.shadows_disabled = !self.shadows_disabled;
        let toggle = if self.shadows_disabled {
            renderer.clear_directional_shadow()
        } else {
            renderer.set_directional_shadow(&demo_shadow_settings())
        };
        match toggle {
            Ok(()) => log::info!(
                "shadows {}",
                if self.shadows_disabled {
                    "cleared"
                } else {
                    "set"
                }
            ),
            Err(shadow_error) => log::warn!("shadow toggle failed: {shadow_error}"),
        }
    }
}
