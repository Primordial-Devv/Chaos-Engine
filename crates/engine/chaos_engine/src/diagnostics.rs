use std::fmt;
use std::mem;
use std::time::Duration;

/// Une mesure nommée du profil de frame : un stage ECS ou un subsystem.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub name: String,
    pub duration: Duration,
}

/// Le profil CPU d'UNE frame — où le temps est dépensé. Obtenu par
/// `EngineContext::diagnostics().last_frame()` : toujours la dernière
/// frame COMPLÈTE, jamais une frame à moitié remplie.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameProfile {
    /// L'index de la frame profilée.
    pub frame_index: u64,
    /// La durée MURALE de la frame (tick à tick, temps réel) — l'attente
    /// de cadence comprise : la vérité du mur, pas celle du travail.
    pub total: Duration,
    /// Le slot de la frame si la boucle est cadencée (`1/target_fps`).
    pub budget: Option<Duration>,
    /// Le TRAVAIL (`update + render`) a-t-il dépassé le budget ? Jamais
    /// le mur : l'attente de cadence n'est pas un dépassement.
    pub over_budget: bool,
    /// Toute la simulation CPU du tick : pas fixes, stages ECS, updates
    /// des subsystems, fin de frame.
    pub update: Duration,
    /// Le rendu CÔTÉ CPU : la somme des hooks `render` (l'encodage et la
    /// soumission — jamais le temps GPU).
    pub render: Duration,
    /// Le nombre de pas fixes exécutés cette frame (la fréquence fixe).
    pub fixed_steps: u32,
    /// La durée du bloc à pas fixe entier.
    pub fixed: Duration,
    /// Le temps par stage ECS variable, dans l'ordre d'exécution.
    pub stages: Vec<Span>,
    /// Le temps d'update par subsystem, dans l'ordre trié.
    pub subsystems: Vec<Span>,
    /// Le temps de render par subsystem, dans l'ordre trié.
    pub renders: Vec<Span>,
}

impl FrameProfile {
    /// Le TRAVAIL CPU de la frame — jamais l'attente de cadence.
    pub fn work(&self) -> Duration {
        self.update + self.render
    }
}

impl fmt::Display for FrameProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frame {}: total {:?}", self.frame_index, self.total)?;
        match self.budget {
            Some(budget) if self.over_budget => write!(f, " (budget {budget:?}, OVER)")?,
            Some(budget) => write!(f, " (budget {budget:?})")?,
            None => {}
        }
        write!(f, ", update {:?}, render {:?}", self.update, self.render)?;
        write!(
            f,
            "\n  fixed: {} step(s) in {:?}",
            self.fixed_steps, self.fixed
        )?;
        for span in &self.stages {
            write!(f, "\n  stage '{}': {:?}", span.name, span.duration)?;
        }
        for span in &self.subsystems {
            write!(f, "\n  update '{}': {:?}", span.name, span.duration)?;
        }
        for span in &self.renders {
            write!(f, "\n  render '{}': {:?}", span.name, span.duration)?;
        }
        Ok(())
    }
}

/// Le service de diagnostics du moteur : un DOUBLE-BUFFER de profils. Le
/// moteur accumule la frame COURANTE pendant qu'elle s'exécute ; au tick
/// suivant, il la clôt (sa durée murale = le `real_delta` mesuré — zéro
/// lecture d'horloge en plus) et l'échange avec le snapshot.
///
/// Coût de la mesure : ~10 paires d'`Instant::now()` par frame (~1 µs),
/// zéro allocation en régime établi (les slots sont réécrits en place —
/// les ensembles de stages et de subsystems sont stables après le
/// démarrage), zéro log sur le chemin chaud (le rapport est à la
/// demande : `Display` de `FrameProfile`).
#[derive(Debug, Default)]
pub struct FrameDiagnostics {
    current: FrameProfile,
    completed: FrameProfile,
    overruns: u64,
    budget: Option<Duration>,
    open: bool,
}

impl FrameDiagnostics {
    /// Le profil de la dernière frame COMPLÈTE — LE snapshot : des
    /// données cohérentes, jamais une frame en cours d'accumulation.
    pub fn last_frame(&self) -> &FrameProfile {
        &self.completed
    }

    /// Le nombre cumulé de frames dont le TRAVAIL a dépassé le budget.
    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub(crate) fn set_budget(&mut self, budget: Option<Duration>) {
        self.budget = budget;
    }

    pub(crate) fn begin_frame(&mut self, frame_index: u64) {
        self.current.frame_index = frame_index;
        self.current.budget = self.budget;
        self.current.over_budget = false;
        self.current.total = Duration::ZERO;
        self.current.update = Duration::ZERO;
        self.current.render = Duration::ZERO;
        self.current.fixed = Duration::ZERO;
        self.current.fixed_steps = 0;
        self.open = true;
    }

    pub(crate) fn record_fixed(&mut self, steps: u32, duration: Duration) {
        self.current.fixed_steps = steps;
        self.current.fixed = duration;
    }

    pub(crate) fn record_stage(&mut self, index: usize, name: &str, duration: Duration) {
        write_span(&mut self.current.stages, index, name, duration);
    }

    pub(crate) fn record_subsystem(&mut self, index: usize, name: &str, duration: Duration) {
        write_span(&mut self.current.subsystems, index, name, duration);
    }

    pub(crate) fn record_render(&mut self, index: usize, name: &str, duration: Duration) {
        write_span(&mut self.current.renders, index, name, duration);
    }

    pub(crate) fn record_update_total(&mut self, duration: Duration) {
        self.current.update = duration;
    }

    pub(crate) fn record_render_total(&mut self, duration: Duration) {
        self.current.render = duration;
    }

    /// Clôt la frame courante avec sa durée murale et l'échange avec le
    /// snapshot. Sans frame ouverte, c'est un no-op (le premier tick n'a
    /// rien à clore).
    pub(crate) fn close_frame(&mut self, total: Duration) {
        if !self.open {
            return;
        }
        self.current.total = total;
        self.current.over_budget = self
            .budget
            .is_some_and(|budget| self.current.work() > budget);
        if self.current.over_budget {
            self.overruns += 1;
        }
        mem::swap(&mut self.current, &mut self.completed);
        self.open = false;
    }
}

fn write_span(list: &mut Vec<Span>, index: usize, name: &str, duration: Duration) {
    if let Some(span) = list.get_mut(index) {
        if span.name != name {
            span.name.clear();
            span.name.push_str(name);
        }
        span.duration = duration;
    } else {
        list.push(Span {
            name: name.to_owned(),
            duration,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_is_the_last_completed_frame() {
        let mut diagnostics = FrameDiagnostics::default();
        diagnostics.begin_frame(1);
        diagnostics.record_stage(0, "update", Duration::from_millis(2));
        diagnostics.record_subsystem(0, "demo", Duration::from_millis(1));
        diagnostics.record_fixed(5, Duration::from_millis(1));
        diagnostics.record_update_total(Duration::from_millis(4));
        diagnostics.close_frame(Duration::from_millis(16));
        let frame = diagnostics.last_frame();
        assert_eq!(frame.frame_index, 1);
        assert_eq!(frame.total, Duration::from_millis(16));
        assert_eq!(frame.fixed_steps, 5);
        assert_eq!(frame.stages[0].name, "update");
        assert_eq!(frame.subsystems[0].name, "demo");
    }

    #[test]
    fn overruns_count_work_against_the_budget_only() {
        let mut diagnostics = FrameDiagnostics::default();
        diagnostics.set_budget(Some(Duration::from_millis(1)));
        diagnostics.begin_frame(1);
        diagnostics.record_update_total(Duration::from_millis(5));
        diagnostics.close_frame(Duration::from_millis(6));
        assert!(diagnostics.last_frame().over_budget);
        assert_eq!(diagnostics.overruns(), 1);
        diagnostics.begin_frame(2);
        diagnostics.record_update_total(Duration::from_micros(100));
        diagnostics.close_frame(Duration::from_millis(1));
        assert!(!diagnostics.last_frame().over_budget);
        assert_eq!(diagnostics.overruns(), 1);
    }

    #[test]
    fn a_budgetless_frame_never_overruns_and_closing_nothing_is_a_no_op() {
        let mut diagnostics = FrameDiagnostics::default();
        diagnostics.close_frame(Duration::from_millis(5));
        assert_eq!(diagnostics.last_frame(), &FrameProfile::default());
        diagnostics.begin_frame(1);
        diagnostics.record_update_total(Duration::from_secs(1));
        diagnostics.close_frame(Duration::from_secs(1));
        assert!(!diagnostics.last_frame().over_budget);
        assert_eq!(diagnostics.overruns(), 0);
    }

    #[test]
    fn span_slots_are_rewritten_in_place() {
        let mut diagnostics = FrameDiagnostics::default();
        diagnostics.begin_frame(1);
        diagnostics.record_stage(0, "update", Duration::from_millis(2));
        diagnostics.close_frame(Duration::from_millis(3));
        diagnostics.begin_frame(2);
        diagnostics.record_stage(0, "update", Duration::from_millis(7));
        diagnostics.close_frame(Duration::from_millis(8));
        let frame = diagnostics.last_frame();
        assert_eq!(frame.stages.len(), 1);
        assert_eq!(frame.stages[0].duration, Duration::from_millis(7));
    }
}
