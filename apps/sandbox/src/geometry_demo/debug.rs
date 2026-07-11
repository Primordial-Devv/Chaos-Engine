//! Le DEBUG RENDERING de la démo : la grille du sol et les axes du
//! monde au spawn (les deux modes de profondeur côte à côte — la grille
//! est occludée par la scène, les axes se voient À TRAVERS), les
//! bounds monde de la ronde (X — le culling visualisé), les frustums de
//! la caméra du miroir et du volume de lumière (F), les lumières
//! dessinées comme données (J), un marqueur de 3 secondes posé à la
//! caméra (T — l'expiration à l'œil), et le toggle global (G). Le rig
//! SOUMET tout chaque frame en immédiat — les CATÉGORIES filtrent au
//! renderer, c'est le mécanisme prouvé en réel.

use chaos_engine::{
    Color, DebugDraw, Renderer, Transform,
    math::{Mat4, Vec3},
};

use super::lighting::Lighting;
use super::mirror::Mirror;
use super::stage::Stage;

/// Les catégories du rig — chacune sa touche, le renderer filtre.
const STAGE_CATEGORY: &str = "demo.stage";
const BOUNDS_CATEGORY: &str = "demo.bounds";
const FRUSTUMS_CATEGORY: &str = "demo.frustums";
const LIGHTS_CATEGORY: &str = "demo.lights";
const MARKERS_CATEGORY: &str = "demo.markers";

/// Le rig debug : SANS état propre — l'activation vit chez le renderer
/// (le toggle global et les catégories), les primitives sont re-soumises
/// chaque frame comme les draws et les lumières.
pub(super) struct DebugRig;

impl DebugRig {
    /// Configure les catégories au spawn : la grille et les axes
    /// VISIBLES (le checkpoint), bounds/frustums/lumières en sommeil —
    /// leurs touches les réveillent.
    pub(super) fn build(renderer: &mut Renderer) -> Self {
        renderer.set_debug_category_enabled(BOUNDS_CATEGORY, false);
        renderer.set_debug_category_enabled(FRUSTUMS_CATEGORY, false);
        renderer.set_debug_category_enabled(LIGHTS_CATEGORY, false);
        Self
    }

    /// Les primitives de la frame : tout est soumis, les catégories
    /// désactivées sont filtrées par le renderer.
    pub(super) fn frame(
        &self,
        renderer: &mut Renderer,
        stage: &Stage,
        mirror: &Mirror,
        lighting: &Lighting,
        ring: &[Transform],
        t: f32,
    ) {
        // La grille du sol (juste au-dessus du damier, testée par la
        // profondeur) et les axes du monde (OVERLAY — visibles à
        // travers la scène : les deux modes côte à côte).
        renderer.queue_debug(
            DebugDraw::grid(Vec3::new(0.0, -0.99, 0.0), 10.0, 1.0)
                .with_color(Color::rgba(0.6, 0.6, 0.7, 0.35))
                .with_category(STAGE_CATEGORY),
        );
        renderer.queue_debug(
            DebugDraw::axes(Mat4::IDENTITY, 2.0)
                .overlay()
                .with_category(STAGE_CATEGORY),
        );
        // Les bounds MONDE de la ronde : les bounds locaux du mesh
        // partagé, transformés par la pose de CHAQUE cube — la matière
        // exacte du frustum culling, rendue visible.
        if let Ok(Some(bounds)) = renderer.mesh_bounds(stage.ring_mesh()) {
            for transform in ring {
                renderer.queue_debug(
                    DebugDraw::aabb(bounds.transformed(transform.matrix()))
                        .with_color(Color::rgb(1.0, 0.85, 0.2))
                        .with_category(BOUNDS_CATEGORY),
                );
            }
        }
        // Les frustums : ce que VOIT le miroir (cyan) et ce que COUVRE
        // le volume d'ombre (ambre) — deux vues qui ne sont pas la
        // caméra principale.
        renderer.queue_debug(
            DebugDraw::frustum(mirror.view_projection())
                .with_color(Color::rgb(0.3, 0.9, 1.0))
                .with_category(FRUSTUMS_CATEGORY),
        );
        renderer.queue_debug(
            DebugDraw::frustum(lighting.shadow_frustum())
                .with_color(Color::rgb(1.0, 0.7, 0.2))
                .with_category(FRUSTUMS_CATEGORY),
        );
        // Les lumières dessinées comme DONNÉES (leurs couleurs) — le
        // soleil ancré au-dessus du centre, les désactivées passées.
        for light in lighting.lights_snapshot(t) {
            if !light.is_enabled() {
                continue;
            }
            renderer.queue_debug(
                DebugDraw::light(&light, Vec3::new(0.0, 5.0, 0.0)).with_category(LIGHTS_CATEGORY),
            );
        }
    }

    /// G : bascule TOUT le debug rendering.
    pub(super) fn toggle_global(&self, renderer: &mut Renderer) {
        let enabled = !renderer.debug_enabled();
        renderer.set_debug_enabled(enabled);
        log::info!(
            "debug rendering {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// X : bascule les bounds de la ronde.
    pub(super) fn toggle_bounds(&self, renderer: &mut Renderer) {
        Self::toggle_category(renderer, BOUNDS_CATEGORY);
    }

    /// F : bascule les frustums (miroir + volume d'ombre).
    pub(super) fn toggle_frustums(&self, renderer: &mut Renderer) {
        Self::toggle_category(renderer, FRUSTUMS_CATEGORY);
    }

    /// J : bascule le dessin des lumières.
    pub(super) fn toggle_lights(&self, renderer: &mut Renderer) {
        Self::toggle_category(renderer, LIGHTS_CATEGORY);
    }

    /// T : pose un marqueur RETENU 3 secondes à la position donnée
    /// (celle de la caméra) — il survit aux `clear_draws` et disparaît
    /// seul : l'expiration prouvée à l'œil.
    pub(super) fn drop_marker(&self, renderer: &mut Renderer, position: Vec3) {
        renderer.queue_debug(
            DebugDraw::marker(position, 0.3)
                .overlay()
                .with_color(Color::rgb(1.0, 0.3, 0.9))
                .with_duration(3.0)
                .with_category(MARKERS_CATEGORY),
        );
        log::info!("debug marker dropped for 3 s at {position}");
    }

    fn toggle_category(renderer: &mut Renderer, category: &str) {
        let enabled = !renderer.debug_category_enabled(category);
        renderer.set_debug_category_enabled(category, enabled);
        log::info!(
            "debug category '{category}' {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }
}
