use std::fmt;
use std::time::Duration;

/// L'état d'un subsystem tel que décidé au démarrage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemState {
    /// Initialisé et tické à chaque frame.
    Active,
    /// Retiré par `runtime.disabled_subsystems` — jamais initialisé.
    Disabled,
    /// Graphique (`requires_graphics`) retiré en mode headless.
    SkippedHeadless,
}

/// Le statut d'un subsystem dans le rapport de santé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemStatus {
    pub name: String,
    pub state: SubsystemState,
}

/// La photo de santé du moteur — cohérente : toutes les jauges sont
/// échantillonnées à la fin de la MÊME frame (`frame_index`). Les
/// compteurs (`errors`/`warnings`) sont CONTINUS : le lecteur diffe deux
/// snapshots pour obtenir le « récent ».
#[derive(Debug, Clone, PartialEq)]
pub struct MetricsSnapshot {
    /// La frame de l'échantillon.
    pub frame_index: u64,
    /// Les images par seconde sur la fenêtre glissante.
    pub fps: f32,
    /// Le temps de frame moyen sur la fenêtre.
    pub frame_time_avg: Duration,
    /// Le temps de frame minimal sur la fenêtre.
    pub frame_time_min: Duration,
    /// Le temps de frame maximal sur la fenêtre.
    pub frame_time_max: Duration,
    /// Les entités vivantes du World.
    pub entities: usize,
    /// Les scènes à l'état actif.
    pub active_scenes: usize,
    /// Les ressources à l'état `Loaded`.
    pub loaded_assets: usize,
    /// Les draws soumis cette frame (0 en headless — pas de renderer).
    pub draw_calls: usize,
    /// Les erreurs comptées par les chemins moteur (cumulatif).
    pub errors: u64,
    /// Les avertissements comptés par les chemins moteur (cumulatif).
    pub warnings: u64,
    /// La mémoire suivie lorsque l'information est disponible — les
    /// octets bruts du cache d'assets aujourd'hui ; `None` avant le
    /// premier échantillon.
    pub tracked_bytes: Option<u64>,
    /// L'état de chaque subsystem enregistré, décidé au démarrage.
    pub subsystems: Vec<SubsystemStatus>,
}

impl fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "health at frame {}: {:.1} fps (avg {:?}, min {:?}, max {:?})",
            self.frame_index,
            self.fps,
            self.frame_time_avg,
            self.frame_time_min,
            self.frame_time_max
        )?;
        write!(
            f,
            "\n  {} entities, {} active scene(s), {} loaded asset(s), {} draw call(s)",
            self.entities, self.active_scenes, self.loaded_assets, self.draw_calls
        )?;
        write!(
            f,
            "\n  {} error(s), {} warning(s)",
            self.errors, self.warnings
        )?;
        if let Some(bytes) = self.tracked_bytes {
            write!(f, ", {bytes} tracked byte(s)")?;
        }
        for status in &self.subsystems {
            write!(f, "\n  subsystem '{}': {:?}", status.name, status.state)?;
        }
        Ok(())
    }
}

/// La fenêtre glissante des temps de frame — 2 secondes à 60 fps.
const WINDOW: usize = 120;

/// Le service des metrics de santé : des indicateurs SIMPLES et CONTINUS
/// (le pendant synthétique du profiling détaillé de `FrameDiagnostics`).
/// Chemin chaud : une écriture de ring buffer + quelques jauges par frame ;
/// les statistiques de fenêtre sont calculées à la LECTURE (froide).
/// Découplé de toute UI : un service en lecture seule du contexte.
pub struct EngineMetrics {
    window: [Duration; WINDOW],
    head: usize,
    filled: usize,
    frame_index: u64,
    entities: usize,
    active_scenes: usize,
    loaded_assets: usize,
    draw_calls: usize,
    errors: u64,
    warnings: u64,
    tracked_bytes: Option<u64>,
    subsystems: Vec<SubsystemStatus>,
}

impl Default for EngineMetrics {
    fn default() -> Self {
        Self {
            window: [Duration::ZERO; WINDOW],
            head: 0,
            filled: 0,
            frame_index: 0,
            entities: 0,
            active_scenes: 0,
            loaded_assets: 0,
            draw_calls: 0,
            errors: 0,
            warnings: 0,
            tracked_bytes: None,
            subsystems: Vec::new(),
        }
    }
}

impl EngineMetrics {
    /// La photo de santé — une passe sur la fenêtre, aucune écriture.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut sum = Duration::ZERO;
        let mut min = Duration::MAX;
        let mut max = Duration::ZERO;
        for &sample in &self.window[..self.filled] {
            sum += sample;
            min = min.min(sample);
            max = max.max(sample);
        }
        let (avg, fps) = if self.filled == 0 {
            (Duration::ZERO, 0.0)
        } else {
            let avg = sum / self.filled as u32;
            let seconds = sum.as_secs_f32();
            let fps = if seconds > 0.0 {
                self.filled as f32 / seconds
            } else {
                0.0
            };
            (avg, fps)
        };
        MetricsSnapshot {
            frame_index: self.frame_index,
            fps,
            frame_time_avg: avg,
            frame_time_min: if self.filled == 0 {
                Duration::ZERO
            } else {
                min
            },
            frame_time_max: max,
            entities: self.entities,
            active_scenes: self.active_scenes,
            loaded_assets: self.loaded_assets,
            draw_calls: self.draw_calls,
            errors: self.errors,
            warnings: self.warnings,
            tracked_bytes: self.tracked_bytes,
            subsystems: self.subsystems.clone(),
        }
    }

    pub(crate) fn record_frame(&mut self, frame_time: Duration) {
        self.window[self.head] = frame_time;
        self.head = (self.head + 1) % WINDOW;
        self.filled = (self.filled + 1).min(WINDOW);
    }

    pub(crate) fn sample(
        &mut self,
        frame_index: u64,
        entities: usize,
        active_scenes: usize,
        loaded_assets: usize,
        draw_calls: usize,
        tracked_bytes: u64,
    ) {
        self.frame_index = frame_index;
        self.entities = entities;
        self.active_scenes = active_scenes;
        self.loaded_assets = loaded_assets;
        self.draw_calls = draw_calls;
        self.tracked_bytes = Some(tracked_bytes);
    }

    pub(crate) fn count_error(&mut self) {
        self.errors += 1;
    }

    pub(crate) fn count_warning(&mut self) {
        self.warnings += 1;
    }

    pub(crate) fn set_subsystems(&mut self, subsystems: Vec<SubsystemStatus>) {
        self.subsystems = subsystems;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_computes_exact_statistics() {
        let mut metrics = EngineMetrics::default();
        metrics.record_frame(Duration::from_millis(10));
        metrics.record_frame(Duration::from_millis(20));
        metrics.record_frame(Duration::from_millis(30));
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.frame_time_avg, Duration::from_millis(20));
        assert_eq!(snapshot.frame_time_min, Duration::from_millis(10));
        assert_eq!(snapshot.frame_time_max, Duration::from_millis(30));
        assert!((snapshot.fps - 50.0).abs() < 0.01);
    }

    #[test]
    fn the_window_keeps_only_the_last_samples() {
        let mut metrics = EngineMetrics::default();
        for _ in 0..WINDOW {
            metrics.record_frame(Duration::from_millis(100));
        }
        for _ in 0..WINDOW {
            metrics.record_frame(Duration::from_millis(10));
        }
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.frame_time_avg, Duration::from_millis(10));
        assert_eq!(snapshot.frame_time_max, Duration::from_millis(10));
    }

    #[test]
    fn counters_accumulate_continuously() {
        let mut metrics = EngineMetrics::default();
        metrics.count_error();
        metrics.count_warning();
        metrics.count_warning();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.warnings, 2);
    }

    #[test]
    fn an_empty_window_yields_a_coherent_snapshot() {
        let metrics = EngineMetrics::default();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.fps, 0.0);
        assert_eq!(snapshot.frame_time_avg, Duration::ZERO);
        assert_eq!(snapshot.frame_time_min, Duration::ZERO);
        assert_eq!(snapshot.frame_time_max, Duration::ZERO);
        assert_eq!(snapshot.tracked_bytes, None);
        assert!(snapshot.subsystems.is_empty());
    }
}
