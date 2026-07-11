//! Les DIAGNOSTICS du renderer : UN snapshot qui explique ce que la
//! dernière frame a rendu, éliminé, possédé et coûté — compteurs
//! exacts dérivés des draws RÉSOLUS (jamais des objets soumis : le
//! coût de l'instrumentation est structurellement borné), coûts CPU
//! mesurés, temps GPU HONNÊTE (`Measured` quand les timestamp queries
//! existent vraiment, `Unavailable` avec sa raison sinon — jamais une
//! valeur inventée). Utilisable sans UI : le snapshot s'affiche
//! (`Display`) en lignes de log lisibles.

use std::fmt;

use crate::lifetime::ResourceStats;

/// Le temps GPU d'une frame — la mesure HONNÊTE : `Measured` vient de
/// vraies timestamp queries (avec quelques frames de latence — la
/// valeur est celle de la dernière frame RÉSOLUE) ; `Unavailable`
/// nomme sa raison (feature absente, mesure pas encore revenue, backend
/// sans mesure). JAMAIS un zéro inventé.
#[derive(Debug, Clone, PartialEq)]
pub enum GpuTiming {
    /// Aucune mesure disponible — la raison est nommée.
    Unavailable {
        /// Pourquoi la mesure n'existe pas.
        reason: String,
    },
    /// Le span GPU mesuré de la dernière frame résolue, en millisecondes.
    Measured {
        /// La durée GPU en millisecondes.
        milliseconds: f32,
    },
}

impl Default for GpuTiming {
    fn default() -> Self {
        Self::Unavailable {
            reason: String::from("no frame rendered yet"),
        }
    }
}

impl fmt::Display for GpuTiming {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { reason } => write!(formatter, "unavailable ({reason})"),
            Self::Measured { milliseconds } => write!(formatter, "{milliseconds:.3} ms"),
        }
    }
}

/// Les TOTAUX de la dernière frame orchestrée — la somme des passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTotals {
    /// Les draws SOUMIS par le consommateur (toutes files, avant
    /// résolution — les écartés et les cullés en font partie).
    pub submitted: usize,
    /// Les OBJETS résolus et rendus (injectés compris — la sémantique
    /// de `PassReport.draws`).
    pub resolved: usize,
    /// Les draws CLASSIQUES soumis au GPU (une instance chacun).
    pub classic_draws: usize,
    /// Les draws INSTANCIÉS soumis au GPU (les batches).
    pub instanced_draws: usize,
    /// Les instances portées par les draws instanciés.
    pub instances: usize,
    /// Les objets REJETÉS par les frustums des passes (le culling).
    pub culled: usize,
    /// Les objets INJECTÉS par le renderer (ciel + primitives debug).
    pub injected: usize,
    /// Les TRIANGLES soumis (indices ou sommets ÷ 3, × instances — le
    /// ciel en compte 1). Les lignes de debug n'y entrent pas.
    pub triangles: usize,
    /// Les SEGMENTS de debug soumis (sommets de lignes ÷ 2).
    pub debug_segments: usize,
    /// Les changements de PIPELINE réellement encodés (dédupliqués par
    /// passe — le miroir de la règle d'encodage du backend).
    pub pipeline_switches: usize,
    /// Les changements de binding MATERIAL réellement encodés (même
    /// règle).
    pub material_switches: usize,
    /// Les passes EXÉCUTÉES (la passe d'ombre non comprise — elle a son
    /// rapport dédié).
    pub passes_executed: usize,
    /// Les passes SAUTÉES (désactivées, cible périmée, surface
    /// indisponible).
    pub passes_skipped: usize,
}

/// Les statistiques d'UNE passe exécutée.
#[derive(Debug, Clone, PartialEq)]
pub struct PassStats {
    /// Le label de la passe.
    pub label: String,
    /// Les objets logiques résolus (injectés compris).
    pub draws: usize,
    /// Les soumissions GPU réelles (batches debug compris).
    pub draw_calls: usize,
    /// Les objets rejetés par le frustum de LA passe.
    pub culled: usize,
    /// Les triangles soumis par la passe (× instances).
    pub triangles: usize,
    /// Les draws classiques.
    pub classic_draws: usize,
    /// Les draws instanciés (batches).
    pub instanced_draws: usize,
    /// Les instances des draws instanciés.
    pub instances: usize,
    /// Les changements de pipeline encodés dans la passe.
    pub pipeline_switches: usize,
    /// Les changements de binding material encodés dans la passe.
    pub material_switches: usize,
    /// Le coût CPU de la RÉSOLUTION de la passe (tri, culling, batching,
    /// debug), en millisecondes.
    pub resolve_cpu_ms: f32,
}

/// Les statistiques de la passe d'OMBRE dérivée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowStats {
    /// Les casters résolus (objets logiques).
    pub draws: usize,
    /// Les soumissions GPU réelles.
    pub draw_calls: usize,
    /// Les tentatives de moisson rejetées par le frustum de lumière.
    pub culled: usize,
    /// Les instances des casters instanciés.
    pub instances: usize,
    /// Les triangles projetés (× instances).
    pub triangles: usize,
}

/// Les coûts CPU de la dernière frame, mesurés (`std::time::Instant`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CpuCost {
    /// La RÉSOLUTION totale (toutes les passes + la moisson d'ombre).
    pub resolve_ms: f32,
    /// L'exécution backend (`GraphicsBackend::render`).
    pub backend_ms: f32,
    /// Le `render_frame` entier.
    pub total_ms: f32,
}

/// Les événements de SURFACE, CUMULÉS depuis la création du renderer —
/// les frames présentées, les erreurs et les récupérations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SurfaceStats {
    /// Les frames dont la présentation a abouti.
    pub presented: u64,
    /// Les présentations sautées : surface INDISPONIBLE (les erreurs).
    pub skipped_unavailable: u64,
    /// Les présentations sautées : surface RECONFIGURÉE (les
    /// récupérations — la frame suivante rend).
    pub reconfigured: u64,
    /// Les présentations sautées : aire nulle (fenêtre minimisée).
    pub zero_area: u64,
}

/// Les FALLBACKS actifs — les chemins dégradés que le renderer assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FallbackStats {
    /// Les permutations de pipeline en échec MÉMOÏSÉ (ciel, ombre,
    /// instancié, debug) — chaque entrée est un chemin dégradé actif.
    pub degraded_permutations: usize,
    /// Les textures fallback builtin vivantes (`chaos.white`, …).
    pub fallback_textures: usize,
    /// Les samplers fallback builtin vivants.
    pub fallback_samplers: usize,
}

/// Le BUDGET CPU du renderer et ses dépassements. `None` (défaut) =
/// jamais de dépassement — le patron du moteur.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BudgetStats {
    /// Le budget CPU par frame en millisecondes (`set_cpu_budget`).
    pub budget_ms: Option<f32>,
    /// Les frames au-dessus du budget, CUMULÉES.
    pub over_budget_frames: u64,
    /// La dernière frame a-t-elle dépassé ?
    pub last_frame_over: bool,
}

/// LE snapshot des diagnostics du renderer — la photo de la DERNIÈRE
/// frame orchestrée (`Renderer::diagnostics`), reconstruite à chaque
/// `render_frame` (vide avant la première ; `render_to_target` n'y
/// touche pas). Les champs de SURFACE et de BUDGET sont cumulatifs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RendererDiagnostics {
    /// Les totaux de la frame.
    pub frame: FrameTotals,
    /// Le détail par passe exécutée, dans l'ordre d'exécution.
    pub passes: Vec<PassStats>,
    /// La passe d'ombre dérivée — `None` sans ombre cette frame.
    pub shadow: Option<ShadowStats>,
    /// Les coûts CPU mesurés.
    pub cpu: CpuCost,
    /// Le temps GPU — mesuré ou indisponible AVEC sa raison.
    pub gpu: GpuTiming,
    /// La photo des ressources possédées (comptes, octets, retraites).
    pub resources: ResourceStats,
    /// Les événements de surface cumulés.
    pub surface: SurfaceStats,
    /// Les chemins dégradés actifs.
    pub fallbacks: FallbackStats,
    /// Le budget CPU et ses dépassements.
    pub budget: BudgetStats,
}

impl fmt::Display for RendererDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = &self.frame;
        writeln!(
            formatter,
            "renderer: {} submitted -> {} resolved ({} culled, {} injected) | {} classic + {} instanced ({} instances) | {} triangles, {} debug segments",
            frame.submitted,
            frame.resolved,
            frame.culled,
            frame.injected,
            frame.classic_draws,
            frame.instanced_draws,
            frame.instances,
            frame.triangles,
            frame.debug_segments
        )?;
        writeln!(
            formatter,
            "  switches: {} pipelines, {} materials | passes: {} executed, {} skipped",
            frame.pipeline_switches,
            frame.material_switches,
            frame.passes_executed,
            frame.passes_skipped
        )?;
        for pass in &self.passes {
            writeln!(
                formatter,
                "  pass '{}': {} draws -> {} draw calls ({} culled) | {} triangles | {} classic + {} instanced ({} instances) | {} + {} switches | resolve {:.3} ms",
                pass.label,
                pass.draws,
                pass.draw_calls,
                pass.culled,
                pass.triangles,
                pass.classic_draws,
                pass.instanced_draws,
                pass.instances,
                pass.pipeline_switches,
                pass.material_switches,
                pass.resolve_cpu_ms
            )?;
        }
        if let Some(shadow) = &self.shadow {
            writeln!(
                formatter,
                "  shadow: {} draws -> {} draw calls ({} culled) | {} triangles ({} instances)",
                shadow.draws, shadow.draw_calls, shadow.culled, shadow.triangles, shadow.instances
            )?;
        }
        writeln!(
            formatter,
            "  cpu: resolve {:.3} ms + backend {:.3} ms = total {:.3} ms | gpu: {}",
            self.cpu.resolve_ms, self.cpu.backend_ms, self.cpu.total_ms, self.gpu
        )?;
        let resources = &self.resources;
        writeln!(
            formatter,
            "  resources: {} buffers, {} textures, {} meshes, {} materials, {} pipelines, {} targets, {} shadow maps | {} retired | ~{} bytes",
            resources.buffers.alive,
            resources.textures.alive,
            resources.meshes,
            resources.materials,
            resources.pipelines,
            resources.render_targets.alive,
            resources.shadow_maps.alive,
            resources.retired,
            resources.estimated_bytes
        )?;
        write!(
            formatter,
            "  surface: {} presented, {} unavailable, {} reconfigured, {} zero-area | fallbacks: {} degraded permutations, {} textures, {} samplers | budget: {} ({} over)",
            self.surface.presented,
            self.surface.skipped_unavailable,
            self.surface.reconfigured,
            self.surface.zero_area,
            self.fallbacks.degraded_permutations,
            self.fallbacks.fallback_textures,
            self.fallbacks.fallback_samplers,
            match self.budget.budget_ms {
                Some(budget) => format!("{budget:.2} ms"),
                None => String::from("none"),
            },
            self.budget.over_budget_frames
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_honest() {
        // Le défaut ne MESURE rien : le GPU est indisponible AVEC sa
        // raison, jamais un zéro inventé ; le budget absent ne dépasse
        // jamais.
        let diagnostics = RendererDiagnostics::default();
        assert!(matches!(
            &diagnostics.gpu,
            GpuTiming::Unavailable { reason } if !reason.is_empty()
        ));
        assert_eq!(diagnostics.budget.budget_ms, None);
        assert_eq!(diagnostics.budget.over_budget_frames, 0);
        assert_eq!(diagnostics.frame, FrameTotals::default());
        assert!(diagnostics.passes.is_empty());
        assert_eq!(diagnostics.shadow, None);
    }

    #[test]
    fn the_display_carries_the_key_numbers() {
        let mut diagnostics = RendererDiagnostics {
            frame: FrameTotals {
                submitted: 504,
                resolved: 505,
                classic_draws: 2,
                instanced_draws: 1,
                instances: 500,
                culled: 7,
                injected: 1,
                triangles: 6036,
                debug_segments: 42,
                pipeline_switches: 3,
                material_switches: 2,
                passes_executed: 2,
                passes_skipped: 1,
            },
            gpu: GpuTiming::Measured {
                milliseconds: 2.345,
            },
            ..RendererDiagnostics::default()
        };
        diagnostics.passes.push(PassStats {
            label: String::from("chaos.main"),
            draws: 505,
            draw_calls: 3,
            culled: 7,
            triangles: 6036,
            classic_draws: 2,
            instanced_draws: 1,
            instances: 500,
            pipeline_switches: 3,
            material_switches: 2,
            resolve_cpu_ms: 0.5,
        });
        let text = diagnostics.to_string();
        assert!(text.contains("504 submitted -> 505 resolved"));
        assert!(text.contains("(7 culled, 1 injected)"));
        assert!(text.contains("2 classic + 1 instanced (500 instances)"));
        assert!(text.contains("6036 triangles"));
        assert!(text.contains("42 debug segments"));
        assert!(text.contains("pass 'chaos.main'"));
        assert!(text.contains("gpu: 2.345 ms"));
        // L'indisponible se SIGNALE, il ne se chiffre pas.
        diagnostics.gpu = GpuTiming::Unavailable {
            reason: String::from("mock"),
        };
        assert!(diagnostics.to_string().contains("gpu: unavailable (mock)"));
    }
}
