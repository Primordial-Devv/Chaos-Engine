//! L'INSTRUMENTATION : l'analyse des draws résolus (le miroir de la
//! règle d'encodage du backend), la clôture des diagnostics de frame,
//! le budget CPU.

use super::*;

impl Renderer {
    /// LE snapshot des diagnostics du renderer : ce que la DERNIÈRE
    /// frame orchestrée a rendu, éliminé, possédé et coûté — compteurs
    /// exacts, coûts CPU mesurés, temps GPU honnête (`Unavailable` avec
    /// sa raison quand la mesure n'existe pas). Reconstruit à chaque
    /// `render_frame` (les champs de surface et de budget sont
    /// CUMULATIFS) ; `render_to_target` n'y touche pas. S'affiche en
    /// lignes de log lisibles (`Display`) — utilisable sans UI.
    pub fn diagnostics(&self) -> &RendererDiagnostics {
        &self.diagnostics
    }

    /// Fixe le BUDGET CPU par frame du renderer, en millisecondes —
    /// `None` (défaut) : jamais de dépassement. Chaque `render_frame`
    /// dont `total_ms` dépasse le budget incrémente
    /// `diagnostics().budget.over_budget_frames`. Une valeur invalide
    /// (non finie ou ≤ 0) est ignorée avec un warn.
    pub fn set_cpu_budget(&mut self, budget_ms: Option<f32>) {
        if let Some(budget) = budget_ms
            && (!budget.is_finite() || budget <= 0.0)
        {
            warn!("cpu budget ignored: it must be finite and positive");
            return;
        }
        self.diagnostics.budget.budget_ms = budget_ms;
    }

    /// L'ANALYSE d'une liste de draws RÉSOLUS — une itération sur les
    /// soumissions GPU (jamais les objets) : classiques/instanciés/
    /// instances/triangles, et les changements d'état EXACTS que le
    /// backend encodera (le MIROIR de sa règle de déduplication
    /// `bound_pipeline`/`bound_material`, batches debug compris).
    pub(super) fn analyze_draws(draws: &[FrameDraw], debug: &[FrameDebugBatch]) -> DrawAnalysis {
        let mut analysis = DrawAnalysis::default();
        let mut bound_pipeline = None;
        let mut bound_material = None;
        for draw in draws {
            let instances = draw.instances.map_or(1, |range| {
                usize::try_from(range.count).unwrap_or(usize::MAX)
            });
            if draw.instances.is_some() {
                analysis.instanced_draws += 1;
                analysis.instances += instances;
            } else {
                analysis.classic_draws += 1;
            }
            analysis.triangles += draw.element_count as usize / 3 * instances;
            if bound_pipeline != Some(draw.pipeline) {
                analysis.pipeline_switches += 1;
                bound_pipeline = Some(draw.pipeline);
            }
            if let Some(binding) = draw.binding
                && bound_material != Some(binding)
            {
                analysis.material_switches += 1;
                bound_material = Some(binding);
            }
        }
        for batch in debug {
            analysis.debug_segments += batch.vertex_count as usize / 2;
            if bound_pipeline != Some(batch.pipeline) {
                analysis.pipeline_switches += 1;
                bound_pipeline = Some(batch.pipeline);
            }
        }
        analysis
    }

    /// Clôt les diagnostics de la frame : les totaux sommés des passes,
    /// les cumulatifs (surface, budget) avancés, les coûts arrêtés, le
    /// temps GPU copié du backend — la photo stockée jusqu'au prochain
    /// `render_frame`. `outcome: None` = rien n'est parti au backend
    /// (plan vide) : aucun événement de surface n'est compté.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_diagnostics(
        &mut self,
        submitted: usize,
        passes: Vec<PassStats>,
        shadow: Option<ShadowDiagnostics>,
        debug_segments: usize,
        resolve_ms: f32,
        backend_ms: f32,
        frame_started: Instant,
        outcome: Option<FrameOutcome>,
    ) {
        let mut totals = FrameTotals {
            submitted,
            debug_segments,
            ..FrameTotals::default()
        };
        for pass in &passes {
            totals.resolved += pass.draws;
            totals.classic_draws += pass.classic_draws;
            totals.instanced_draws += pass.instanced_draws;
            totals.instances += pass.instances;
            totals.culled += pass.culled;
            totals.triangles += pass.triangles;
            totals.pipeline_switches += pass.pipeline_switches;
            totals.material_switches += pass.material_switches;
        }
        for report in &self.report.passes {
            totals.injected += report.breakdown.injected;
            if report.outcome == PassOutcome::Executed {
                totals.passes_executed += 1;
            } else {
                totals.passes_skipped += 1;
            }
        }
        match outcome {
            Some(FrameOutcome::Rendered) => self.diagnostics.surface.presented += 1,
            Some(FrameOutcome::Skipped(reason)) => match reason {
                crate::frame::FrameSkipReason::SurfaceUnavailable => {
                    self.diagnostics.surface.skipped_unavailable += 1;
                }
                crate::frame::FrameSkipReason::SurfaceReconfigured => {
                    self.diagnostics.surface.reconfigured += 1;
                }
                crate::frame::FrameSkipReason::ZeroArea => {
                    self.diagnostics.surface.zero_area += 1;
                }
            },
            None => {}
        }
        let total_ms = frame_started.elapsed().as_secs_f32() * 1000.0;
        let over_budget = self
            .diagnostics
            .budget
            .budget_ms
            .is_some_and(|budget| total_ms > budget);
        self.diagnostics.budget.last_frame_over = over_budget;
        if over_budget {
            self.diagnostics.budget.over_budget_frames += 1;
        }
        // Les fallbacks ACTIFS : chaque permutation en échec mémoïsé est
        // un chemin dégradé assumé, les builtins vivants sont les filets.
        let degraded = self
            .sky_pipelines
            .values()
            .filter(|pipeline| pipeline.is_none())
            .count()
            + self
                .shadow_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count()
            + self
                .instanced_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count()
            + self
                .debug_pipelines
                .values()
                .filter(|pipeline| pipeline.is_none())
                .count();
        self.diagnostics.fallbacks = FallbackStats {
            degraded_permutations: degraded,
            fallback_textures: self.lifetime.fallback_texture_count(),
            fallback_samplers: self.lifetime.fallback_sampler_count(),
        };
        self.diagnostics.frame = totals;
        self.diagnostics.passes = passes;
        self.diagnostics.shadow = shadow;
        self.diagnostics.cpu = CpuCost {
            resolve_ms,
            backend_ms,
            total_ms,
        };
        self.diagnostics.gpu = self.backend.gpu_frame_time();
        self.diagnostics.resources = self.resource_stats();
    }
}
