use std::time::{Duration, Instant};

/// Instantané temporel d'une frame, transmis aux systèmes à chaque update.
///
/// Deux temps distincts, une frontière nette :
/// - **le temps de JEU** (`delta`, `elapsed`) — clampé (jamais de pas
///   géant) puis ÉCHELONNÉ (`scale`) : c'est lui qui pilote la simulation ;
/// - **le temps RÉEL** (`real_delta`, `real_elapsed`) — brut, ni clampé ni
///   échelonné : la vérité murale, pour le profiling et les timeouts (un
///   breakpoint de 5 s montre 5 s réelles).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Time {
    /// Delta de JEU : clampé puis échelonné.
    pub delta: Duration,
    /// Temps de JEU écoulé (l'accumulation des deltas de jeu).
    pub elapsed: Duration,
    pub frame_index: u64,
    /// Delta RÉEL : brut, non clampé, non échelonné.
    pub real_delta: Duration,
    /// Temps RÉEL écoulé depuis le démarrage de l'horloge.
    pub real_elapsed: Duration,
    /// L'échelle appliquée à cette frame (1.0 = temps normal).
    pub scale: f32,
}

impl Time {
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// Horloge de frame : mesure le delta réel, le borne (un gel — breakpoint,
/// mise en veille — ne produit jamais un pas de temps de jeu géant) puis
/// l'échelonne. Le temps réel, lui, reste brut.
#[derive(Debug)]
pub struct FrameClock {
    last_tick: Instant,
    max_delta: Duration,
    scale: f32,
    time: Time,
}

impl FrameClock {
    pub const DEFAULT_MAX_DELTA: Duration = Duration::from_millis(250);

    pub fn new() -> Self {
        Self::with_max_delta(Self::DEFAULT_MAX_DELTA)
    }

    pub fn with_max_delta(max_delta: Duration) -> Self {
        Self {
            last_tick: Instant::now(),
            max_delta,
            scale: 1.0,
            time: Time {
                scale: 1.0,
                ..Time::default()
            },
        }
    }

    /// Fixe l'échelle de temps (slow-motion, accéléré). Assainie : une
    /// valeur non finie est IGNORÉE, une valeur négative devient 0.
    pub fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() {
            self.scale = scale.max(0.0);
        }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn tick(&mut self) -> Time {
        self.tick_at(Instant::now())
    }

    /// Resynchronise l'horloge après un gel VOLONTAIRE (la reprise de
    /// pause) : l'écart depuis le dernier tick est crédité au temps RÉEL
    /// (`real_elapsed` — la vérité murale est conservée) mais ne produira
    /// AUCUN delta — le prochain tick mesure depuis maintenant. Zéro saut
    /// de simulation, zéro rafale de pas fixes ; le clamp `max_delta`
    /// reste le filet des gels involontaires (breakpoint, machine).
    pub fn resync(&mut self) {
        self.resync_at(Instant::now());
    }

    fn resync_at(&mut self, now: Instant) {
        let gap = now.saturating_duration_since(self.last_tick);
        self.time.real_elapsed += gap;
        self.last_tick = now;
    }

    fn tick_at(&mut self, now: Instant) -> Time {
        let raw = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let delta = raw.min(self.max_delta).mul_f64(f64::from(self.scale));
        self.time = Time {
            delta,
            elapsed: self.time.elapsed + delta,
            frame_index: self.time.frame_index + 1,
            real_delta: raw,
            real_elapsed: self.time.real_elapsed + raw,
            scale: self.scale,
        };
        self.time
    }

    pub fn time(&self) -> Time {
        self.time
    }
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Instantané du temps FIXE — la ressource des simulations déterministes
/// (future physique en tête). `delta` est LE pas, constant : la logique à
/// pas fixe ne lit jamais `Time` pour avancer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FixedTime {
    /// LE pas fixe — constant d'un pas à l'autre.
    pub delta: Duration,
    /// Temps fixe accumulé : `step_index × delta`.
    pub elapsed: Duration,
    /// Numéro du pas.
    pub step_index: u64,
}

impl FixedTime {
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// L'accumulateur du pas fixe : transforme les deltas de JEU (clampés +
/// échelonnés) en un nombre de pas fixes à exécuter, BORNÉ par frame —
/// l'anti-spirale de la mort : au-delà du rattrapage permis, l'excédent
/// d'accumulateur est ABANDONNÉ (sous surcharge, la simulation ralentit au
/// lieu de spiraler — le choix standard). Déterministe : deux horloges
/// nourries des mêmes deltas produisent les mêmes séquences de pas.
#[derive(Debug)]
pub struct FixedClock {
    step: Duration,
    accumulator: Duration,
    max_steps_per_frame: u32,
    time: FixedTime,
}

impl FixedClock {
    /// 60 pas par seconde — le pas par défaut du moteur.
    pub const DEFAULT_STEP: Duration = Duration::from_nanos(1_000_000_000 / 60);
    /// Le rattrapage maximal par frame — l'anti-spirale.
    pub const DEFAULT_MAX_STEPS: u32 = 5;

    pub fn new(step: Duration) -> Self {
        Self {
            step,
            accumulator: Duration::ZERO,
            max_steps_per_frame: Self::DEFAULT_MAX_STEPS,
            time: FixedTime {
                delta: step,
                ..FixedTime::default()
            },
        }
    }

    pub fn with_max_steps(mut self, max_steps_per_frame: u32) -> Self {
        self.max_steps_per_frame = max_steps_per_frame;
        self
    }

    pub fn step_duration(&self) -> Duration {
        self.step
    }

    /// Accumule un delta de jeu et rend le nombre de pas à exécuter cette
    /// frame — borné ; l'excédent au-delà du rattrapage est abandonné.
    pub fn advance(&mut self, game_delta: Duration) -> u32 {
        if self.step.is_zero() {
            return 0;
        }
        self.accumulator += game_delta;
        let mut steps = 0;
        while self.accumulator >= self.step && steps < self.max_steps_per_frame {
            self.accumulator -= self.step;
            steps += 1;
        }
        if self.accumulator > self.step {
            self.accumulator = self.step;
        }
        steps
    }

    /// Avance d'UN pas et rend l'instantané — à appeler exactement le
    /// nombre de fois rendu par [`FixedClock::advance`].
    pub fn step(&mut self) -> FixedTime {
        self.time = FixedTime {
            delta: self.step,
            elapsed: self.time.elapsed + self.step,
            step_index: self.time.step_index + 1,
        };
        self.time
    }

    pub fn time(&self) -> FixedTime {
        self.time
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::new(Self::DEFAULT_STEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_advances_time() {
        let mut clock = FrameClock::new();
        let start = clock.last_tick;
        let first = clock.tick_at(start + Duration::from_millis(16));
        assert_eq!(first.delta, Duration::from_millis(16));
        assert_eq!(first.elapsed, Duration::from_millis(16));
        assert_eq!(first.frame_index, 1);
        let second = clock.tick_at(start + Duration::from_millis(32));
        assert_eq!(second.delta, Duration::from_millis(16));
        assert_eq!(second.elapsed, Duration::from_millis(32));
        assert_eq!(second.frame_index, 2);
    }

    #[test]
    fn delta_is_clamped() {
        let mut clock = FrameClock::new();
        let start = clock.last_tick;
        let time = clock.tick_at(start + Duration::from_secs(5));
        assert_eq!(time.delta, FrameClock::DEFAULT_MAX_DELTA);
        assert_eq!(time.elapsed, FrameClock::DEFAULT_MAX_DELTA);
    }

    #[test]
    fn stalled_clock_yields_zero_delta() {
        let mut clock = FrameClock::new();
        let start = clock.last_tick;
        let time = clock.tick_at(start);
        assert_eq!(time.delta, Duration::ZERO);
        assert_eq!(time.frame_index, 1);
    }

    #[test]
    fn resync_swallows_the_gap_without_a_delta_jump() {
        let mut clock = FrameClock::new();
        let start = clock.last_tick;
        clock.tick_at(start + Duration::from_millis(16));
        clock.resync_at(start + Duration::from_secs(10));
        let time = clock.tick_at(start + Duration::from_secs(10) + Duration::from_millis(16));
        assert_eq!(time.delta, Duration::from_millis(16));
        assert_eq!(time.real_delta, Duration::from_millis(16));
        assert_eq!(time.elapsed, Duration::from_millis(32));
        assert_eq!(
            time.real_elapsed,
            Duration::from_secs(10) + Duration::from_millis(16)
        );
        assert_eq!(time.frame_index, 2);
    }

    #[test]
    fn the_tick_separates_real_and_game_time() {
        let mut clock = FrameClock::new();
        clock.set_scale(0.5);
        let start = clock.last_tick;
        let time = clock.tick_at(start + Duration::from_millis(20));
        assert_eq!(time.real_delta, Duration::from_millis(20));
        assert_eq!(time.delta, Duration::from_millis(10));
        assert_eq!(time.elapsed, Duration::from_millis(10));
        assert_eq!(time.real_elapsed, Duration::from_millis(20));
        assert_eq!(time.scale, 0.5);
    }

    #[test]
    fn scale_zero_freezes_game_time_but_not_real_time() {
        let mut clock = FrameClock::new();
        clock.set_scale(0.0);
        let start = clock.last_tick;
        let time = clock.tick_at(start + Duration::from_millis(16));
        assert_eq!(time.delta, Duration::ZERO);
        assert_eq!(time.elapsed, Duration::ZERO);
        assert_eq!(time.real_elapsed, Duration::from_millis(16));
        assert_eq!(time.frame_index, 1);
    }

    #[test]
    fn an_invalid_scale_is_sanitized() {
        let mut clock = FrameClock::new();
        clock.set_scale(-2.0);
        assert_eq!(clock.scale(), 0.0);
        clock.set_scale(1.5);
        clock.set_scale(f32::NAN);
        assert_eq!(clock.scale(), 1.5);
        clock.set_scale(f32::INFINITY);
        assert_eq!(clock.scale(), 1.5);
    }

    #[test]
    fn real_delta_is_never_clamped() {
        let mut clock = FrameClock::new();
        let start = clock.last_tick;
        let time = clock.tick_at(start + Duration::from_secs(5));
        assert_eq!(time.delta, FrameClock::DEFAULT_MAX_DELTA);
        assert_eq!(time.real_delta, Duration::from_secs(5));
    }

    #[test]
    fn the_accumulator_yields_exact_steps() {
        let mut fixed = FixedClock::new(Duration::from_millis(10));
        assert_eq!(fixed.advance(Duration::from_millis(25)), 2);
        assert_eq!(fixed.advance(Duration::from_millis(7)), 1);
        assert_eq!(fixed.advance(Duration::from_millis(0)), 0);
        assert_eq!(fixed.advance(Duration::from_millis(8)), 1);
    }

    #[test]
    fn steps_are_capped_and_the_excess_is_dropped() {
        let mut fixed = FixedClock::new(Duration::from_millis(10)).with_max_steps(4);
        assert_eq!(fixed.advance(Duration::from_millis(100)), 4);
        assert_eq!(fixed.advance(Duration::from_millis(0)), 1);
        assert_eq!(fixed.advance(Duration::from_millis(0)), 0);
    }

    #[test]
    fn fixed_time_advances_deterministically() {
        let mut fixed = FixedClock::new(Duration::from_millis(10));
        let first = fixed.step();
        let second = fixed.step();
        assert_eq!(first.delta, Duration::from_millis(10));
        assert_eq!(first.step_index, 1);
        assert_eq!(second.elapsed, Duration::from_millis(20));
        assert_eq!(second.step_index, 2);
    }

    #[test]
    fn identical_deltas_yield_identical_step_sequences() {
        let feed = [
            Duration::from_millis(16),
            Duration::from_millis(33),
            Duration::from_millis(5),
            Duration::from_millis(120),
            Duration::from_millis(9),
        ];
        let run = || {
            let mut fixed = FixedClock::new(Duration::from_millis(10));
            feed.iter()
                .map(|delta| {
                    let steps = fixed.advance(*delta);
                    for _ in 0..steps {
                        fixed.step();
                    }
                    (steps, fixed.time().step_index)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
