use std::time::{Duration, Instant};

use chaos_core::{ChaosError, ChaosResult, Event, FrameClock, WindowEvent};
use chaos_window::{WindowEventHandler, WindowHandle, run_event_loop};
use log::{debug, error, info, trace};

use crate::config::EngineConfig;
use crate::context::EngineContext;
use crate::render_subsystem::RenderSubsystem;
use crate::subsystem::Subsystem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineState {
    Created,
    Running,
    Stopped,
}

/// Cœur du moteur : possède le cycle de vie, la boucle logique et les subsystems.
///
/// La boucle d'événements OS appartient à chaos_window (exigence macOS) ; le
/// moteur reçoit la vie via les hooks de `WindowEventHandler`.
pub struct Engine {
    config: EngineConfig,
    state: EngineState,
    context: EngineContext,
    clock: FrameClock,
    subsystems: Vec<Box<dyn Subsystem>>,
    initialized: usize,
    window: Option<WindowHandle>,
    init_error: Option<ChaosError>,
    target_frame_time: Option<Duration>,
    next_frame_at: Option<Instant>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        let target_frame_time = config
            .target_fps
            .filter(|fps| *fps > 0)
            .map(|fps| Duration::from_secs(1) / fps);
        Self {
            config,
            state: EngineState::Created,
            context: EngineContext::default(),
            clock: FrameClock::new(),
            subsystems: Vec::new(),
            initialized: 0,
            window: None,
            init_error: None,
            target_frame_time,
            next_frame_at: None,
        }
    }

    /// Enregistre un subsystem ; l'ordre d'enregistrement définit l'ordre d'init.
    pub fn add_subsystem(&mut self, subsystem: Box<dyn Subsystem>) -> &mut Self {
        self.subsystems.push(subsystem);
        self
    }

    /// Démarre le moteur et bloque jusqu'à l'arrêt propre.
    /// Doit être appelé depuis le main thread, une seule fois par processus :
    /// les OS ne permettent pas de recréer la boucle d'événements.
    pub fn run(&mut self) -> ChaosResult<()> {
        info!(
            "{} starting (Chaos Engine {})",
            self.config.app_name,
            env!("CARGO_PKG_VERSION")
        );
        run_event_loop(self.config.window.clone(), self)?;
        if let Some(init_error) = self.init_error.take() {
            return Err(init_error);
        }
        info!("{} stopped cleanly", self.config.app_name);
        Ok(())
    }

    fn start(&mut self) {
        for index in 0..self.subsystems.len() {
            let name = self.subsystems[index].name().to_owned();
            debug!("init subsystem '{name}'");
            if let Err(init_error) = self.subsystems[index].init(&mut self.context) {
                error!("subsystem '{name}' failed to init: {init_error}");
                self.init_error = Some(init_error);
                self.context.request_exit();
                return;
            }
            self.initialized = index + 1;
        }
        self.clock = FrameClock::new();
        self.state = EngineState::Running;
        info!("engine running ({} subsystem(s))", self.subsystems.len());
    }
}

impl WindowEventHandler for Engine {
    fn on_window_ready(&mut self, window: WindowHandle) {
        let (width, height) = window.inner_size();
        info!(
            "window ready: {width}x{height} (scale factor {})",
            window.scale_factor()
        );
        self.subsystems.push(Box::new(RenderSubsystem::new(
            window.clone(),
            self.config.clear_color,
            self.config.vsync,
        )));
        self.window = Some(window);
        self.start();
    }

    fn on_event(&mut self, event: Event) {
        trace!("event: {event:?}");
        if event == Event::Window(WindowEvent::CloseRequested) {
            info!("close requested by the system");
            self.context.request_exit();
        }
        for subsystem in &mut self.subsystems[..self.initialized] {
            subsystem.on_event(&event, &mut self.context);
        }
    }

    fn on_update(&mut self) {
        if self.state != EngineState::Running || self.context.exit_requested() {
            return;
        }
        let now = Instant::now();
        if let Some(next_frame_at) = self.next_frame_at
            && now < next_frame_at
        {
            return;
        }
        let time = self.clock.tick();
        self.context.set_time(time);
        for subsystem in &mut self.subsystems[..self.initialized] {
            subsystem.update(&mut self.context);
        }
        if let Some(limit) = self.config.frame_limit
            && time.frame_index >= limit
        {
            info!("frame limit reached ({limit}), requesting exit");
            self.context.request_exit();
        }
        if !self.context.exit_requested() {
            if let Some(target) = self.target_frame_time {
                self.next_frame_at = Some(now + target);
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn on_redraw(&mut self) {
        if self.state != EngineState::Running || self.context.exit_requested() {
            return;
        }
        for subsystem in &mut self.subsystems[..self.initialized] {
            subsystem.render(&mut self.context);
        }
    }

    fn frame_deadline(&self) -> Option<Instant> {
        if self.state != EngineState::Running || self.context.exit_requested() {
            return None;
        }
        self.next_frame_at
    }

    fn exit_requested(&self) -> bool {
        self.context.exit_requested()
    }

    fn on_shutdown(&mut self) {
        if self.state == EngineState::Stopped {
            return;
        }
        info!("engine shutting down");
        for subsystem in self.subsystems[..self.initialized].iter_mut().rev() {
            debug!("shutdown subsystem '{}'", subsystem.name());
            subsystem.shutdown(&mut self.context);
        }
        self.initialized = 0;
        self.window = None;
        self.state = EngineState::Stopped;
        info!("engine stopped");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use chaos_core::{ElementState, InputEvent, KeyCode};

    use super::*;

    #[derive(Clone, Default)]
    struct Journal(Rc<RefCell<Vec<String>>>);

    impl Journal {
        fn push(&self, entry: impl Into<String>) {
            self.0.borrow_mut().push(entry.into());
        }

        fn entries(&self) -> Vec<String> {
            self.0.borrow().clone()
        }
    }

    struct Probe {
        name: &'static str,
        journal: Journal,
        fail_init: bool,
    }

    impl Probe {
        fn boxed(name: &'static str, journal: &Journal) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: false,
            })
        }

        fn failing(name: &'static str, journal: &Journal) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: true,
            })
        }
    }

    impl Subsystem for Probe {
        fn name(&self) -> &str {
            self.name
        }

        fn init(&mut self, _context: &mut EngineContext) -> ChaosResult<()> {
            self.journal.push(format!("init {}", self.name));
            if self.fail_init {
                return Err(ChaosError::Engine(format!("{} init failed", self.name)));
            }
            Ok(())
        }

        fn on_event(&mut self, _event: &Event, _context: &mut EngineContext) {
            self.journal.push(format!("event {}", self.name));
        }

        fn update(&mut self, context: &mut EngineContext) {
            self.journal.push(format!(
                "update {} {}",
                self.name,
                context.time().frame_index
            ));
        }

        fn render(&mut self, _context: &mut EngineContext) {
            self.journal.push(format!("render {}", self.name));
        }

        fn shutdown(&mut self, _context: &mut EngineContext) {
            self.journal.push(format!("shutdown {}", self.name));
        }
    }

    fn unpaced_config() -> EngineConfig {
        EngineConfig {
            target_fps: None,
            ..EngineConfig::default()
        }
    }

    #[test]
    fn init_in_order_then_shutdown_in_reverse() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.add_subsystem(Probe::boxed("b", &journal));
        engine.start();
        engine.on_shutdown();
        assert_eq!(
            journal.entries(),
            vec!["init a", "init b", "shutdown b", "shutdown a"]
        );
    }

    #[test]
    fn close_request_triggers_exit() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        assert!(!engine.exit_requested());
        engine.on_event(Event::Window(WindowEvent::CloseRequested));
        assert!(engine.exit_requested());
    }

    #[test]
    fn events_reach_subsystems() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_event(Event::Input(InputEvent::Keyboard {
            key: KeyCode::Escape,
            state: ElementState::Pressed,
            repeat: false,
        }));
        assert!(journal.entries().contains(&String::from("event a")));
    }

    #[test]
    fn frame_limit_requests_exit() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            frame_limit: Some(3),
            ..unpaced_config()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        for _ in 0..3 {
            engine.on_update();
        }
        assert!(engine.exit_requested());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2", "update a 3"]
        );
    }

    #[test]
    fn failed_init_shuts_down_only_initialized_subsystems() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.add_subsystem(Probe::failing("b", &journal));
        engine.add_subsystem(Probe::boxed("c", &journal));
        engine.start();
        assert!(engine.exit_requested());
        assert!(engine.init_error.is_some());
        engine.on_shutdown();
        assert_eq!(journal.entries(), vec!["init a", "init b", "shutdown a"]);
    }

    #[test]
    fn render_runs_after_update() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_redraw();
        assert_eq!(journal.entries(), vec!["init a", "update a 1", "render a"]);
    }

    #[test]
    fn paced_update_is_gated() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            target_fps: Some(60),
            ..EngineConfig::default()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_update();
        assert_eq!(journal.entries(), vec!["init a", "update a 1"]);
    }

    #[test]
    fn redraw_before_start_is_ignored() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.on_redraw();
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn update_before_start_is_ignored() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.on_update();
        assert!(journal.entries().is_empty());
    }
}
