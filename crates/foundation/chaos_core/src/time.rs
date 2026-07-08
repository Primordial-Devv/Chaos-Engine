use std::time::{Duration, Instant};

/// Instantané temporel d'une frame, transmis aux systèmes à chaque update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Time {
    pub delta: Duration,
    pub elapsed: Duration,
    pub frame_index: u64,
}

impl Time {
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// Horloge de frame : mesure le delta réel et le borne, afin qu'un gel
/// (breakpoint, mise en veille) ne produise jamais un pas de temps géant.
#[derive(Debug)]
pub struct FrameClock {
    last_tick: Instant,
    max_delta: Duration,
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
            time: Time::default(),
        }
    }

    pub fn tick(&mut self) -> Time {
        self.tick_at(Instant::now())
    }

    fn tick_at(&mut self, now: Instant) -> Time {
        let raw = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let delta = raw.min(self.max_delta);
        self.time = Time {
            delta,
            elapsed: self.time.elapsed + delta,
            frame_index: self.time.frame_index + 1,
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
}
