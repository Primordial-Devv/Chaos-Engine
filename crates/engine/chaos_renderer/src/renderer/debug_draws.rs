//! Le DEBUG RENDERING côté renderer : le store (frame/retenues), la
//! soumission validée, le temps fourni par le consommateur, les
//! toggles, et la résolution par passe en batches Scene/Overlay.

use super::*;

impl Renderer {
    /// Résout le DEBUG d'une passe : les primitives visibles (toggle
    /// global, catégorie, passe cible — `None` vise la principale) sont
    /// tessellées en DEUX plages d'un même tableau de sommets — Scene
    /// (testée) puis Overlay (par-dessus tout, dessinée en dernier) —
    /// et chaque plage non vide devient un batch avec sa permutation.
    /// Rend (batches, sommets, primitives DESSINÉES) — le compte nourrit
    /// `injected` (la règle du ciel).
    pub(super) fn resolve_pass_debug(
        debug: &DebugStore,
        context: &mut PipelineContext<'_>,
        pass_index: usize,
        color_format: Option<TextureFormat>,
    ) -> (Vec<FrameDebugBatch>, Vec<DebugVertex>, usize) {
        if !debug.enabled {
            return (Vec::new(), Vec::new(), 0);
        }
        let mut scene = Vec::new();
        let mut overlay = Vec::new();
        let mut scene_count = 0usize;
        let mut overlay_count = 0usize;
        let draws = debug
            .frame
            .iter()
            .chain(debug.retained.iter().map(|retained| &retained.draw));
        for draw in draws {
            if debug.disabled_categories.contains(&draw.category) {
                continue;
            }
            let target = draw.pass.map_or(MAIN_PASS, |pass| pass.0 as usize);
            if target != pass_index {
                continue;
            }
            match draw.depth {
                DebugDepth::Scene => {
                    draw.shape.tessellate(draw.color, &mut scene);
                    scene_count += 1;
                }
                DebugDepth::Overlay => {
                    draw.shape.tessellate(draw.color, &mut overlay);
                    overlay_count += 1;
                }
            }
        }
        let mut batches = Vec::new();
        let mut vertices = Vec::new();
        let mut primitives = 0;
        for (mode_vertices, count, depth) in [
            (scene, scene_count, DebugDepth::Scene),
            (overlay, overlay_count, DebugDepth::Overlay),
        ] {
            if mode_vertices.is_empty() {
                continue;
            }
            let Some(pipeline) = Self::resolve_debug_pipeline(context, color_format, depth) else {
                continue;
            };
            let first_vertex = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            let vertex_count = u32::try_from(mode_vertices.len()).unwrap_or(u32::MAX);
            vertices.extend(mode_vertices);
            batches.push(FrameDebugBatch {
                pipeline,
                first_vertex,
                vertex_count,
            });
            primitives += count;
        }
        (batches, vertices, primitives)
    }

    /// Soumet une primitive de DEBUG — le pendant visuel de
    /// `queue_draw` pour les données spatiales. Durée `0` (défaut) : la
    /// primitive vit la frame de simulation courante, vidée par
    /// `clear_draws` ; durée `> 0` : RETENUE, elle survit à
    /// `clear_draws` et expire seule au fil de `advance_debug_time`.
    /// Une primitive INVALIDE (géométrie non finie, taille nulle,
    /// frustum non inversible, passe inconnue…) est écartée ICI avec un
    /// warn — jamais stockée, jamais envoyée au GPU.
    pub fn queue_debug(&mut self, draw: DebugDraw) {
        if let Err(refusal) = draw.validate() {
            warn!("{refusal}");
            return;
        }
        if let Some(pass) = draw.pass
            && self.passes.get(pass.0 as usize).is_none()
        {
            warn!("debug draw refused: unknown render pass handle");
            return;
        }
        if draw.duration > 0.0 {
            self.debug.retained.push(RetainedDebugDraw {
                remaining: draw.duration,
                draw,
            });
        } else {
            self.debug.frame.push(draw);
        }
    }

    /// Avance le temps du debug rendering : les primitives RETENUES
    /// décomptent `delta_seconds` et expirent seules. Le renderer n'a
    /// pas d'horloge — le CONSOMMATEUR fournit le temps (la démo le
    /// fait à chaque update). Un delta invalide (non fini, négatif) est
    /// ignoré avec un warn.
    pub fn advance_debug_time(&mut self, delta_seconds: f32) {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            warn!("debug time ignored: delta must be finite and non-negative");
            return;
        }
        self.debug.retained.retain_mut(|retained| {
            retained.remaining -= delta_seconds;
            retained.remaining > 0.0
        });
    }

    /// Active ou désactive TOUT le debug rendering (activé par défaut).
    /// Les soumissions restent acceptées et les retenues continuent
    /// d'expirer — le toggle filtre au RENDU seulement.
    pub fn set_debug_enabled(&mut self, enabled: bool) {
        self.debug.enabled = enabled;
    }

    /// Le debug rendering est-il activé ?
    pub fn debug_enabled(&self) -> bool {
        self.debug.enabled
    }

    /// Active ou désactive une CATÉGORIE de debug (toutes activées par
    /// défaut). Le filtre agit au RENDU : les retenues d'une catégorie
    /// désactivée continuent d'expirer et réapparaissent au réveil.
    pub fn set_debug_category_enabled(&mut self, category: &str, enabled: bool) {
        if enabled {
            self.debug.disabled_categories.remove(category);
        } else {
            self.debug
                .disabled_categories
                .insert(String::from(category));
        }
    }

    /// La catégorie de debug est-elle activée ?
    pub fn debug_category_enabled(&self, category: &str) -> bool {
        !self.debug.disabled_categories.contains(category)
    }

    /// L'inspection du store de debug : les comptes frame/retenues.
    pub fn debug_stats(&self) -> DebugStats {
        DebugStats {
            frame: self.debug.frame.len(),
            retained: self.debug.retained.len(),
        }
    }
}
