use std::time::{Duration, Instant};

use chaos_core::{ChaosError, ChaosResult, Event, FixedClock, FrameClock, WindowEvent};
use chaos_renderer::{Renderer, RendererConfig};
use chaos_scene::TransformPropagation;
use chaos_window::{WindowEventHandler, WindowHandle, run_event_loop};
use log::{debug, error, info, trace, warn};

use crate::config::EngineConfig;
use crate::context::EngineContext;
use crate::metrics::{SubsystemState, SubsystemStatus};
use crate::render_subsystem::RenderSubsystem;
use crate::subsystem::Subsystem;

/// Les phases moteur du Schedule ECS : la POLITIQUE des noms vit ici, le
/// mécanisme dans chaos_ecs. L'ordre garanti : UPDATE → LATE_UPDATE →
/// POST_UPDATE, avant les updates des subsystems — chaque système futur
/// sait précisément à quel moment il s'exécute. Le modèle de frame
/// complet : `docs/architecture/engine-loop.md`.
pub mod stages {
    /// LA SIMULATION À PAS FIXE : les systèmes déterministes (future
    /// physique en tête) — exécutés 0..N fois par frame (rattrapage
    /// borné, anti-spirale), AVANT la simulation variable. Ils lisent la
    /// ressource `FixedTime` (le pas constant), jamais `Time`. Ce stage
    /// vit dans le schedule FIXE (`fixed_schedule_mut`), pas le variable.
    pub const FIXED_UPDATE: &str = "fixed_update";

    /// LA SIMULATION : les systèmes de jeu et de contenu — ils lisent les
    /// événements de la frame (messages) et mutent l'état du monde
    /// (transforms LOCAUX compris).
    pub const UPDATE: &str = "update";

    /// LES MISES À JOUR TARDIVES : les systèmes qui réagissent à la
    /// simulation (caméra qui suit, contraintes) — après TOUTES les
    /// mutations de jeu, mais AVANT la propagation : leurs écritures de
    /// transforms sont propagées la même frame.
    pub const LATE_UPDATE: &str = "late_update";

    /// LA PROPAGATION : les calculs dérivés (les `GlobalTransform` par
    /// `TransformPropagation`, service moteur) — le dernier stage ; les
    /// subsystems lisent ensuite l'état propagé, frais.
    pub const POST_UPDATE: &str = "post_update";
}

/// La machine d'états du moteur : `Created → Running ⇄ Paused → Stopped`.
/// Les transitions invalides sont refusées explicitement, jamais des
/// effets silencieux ; le détail : `docs/architecture/engine-loop.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineState {
    Created,
    Running,
    Paused,
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
    fixed_clock: FixedClock,
    subsystems: Vec<Box<dyn Subsystem>>,
    initialized: usize,
    window: Option<WindowHandle>,
    auto_paused: bool,
    suspended: bool,
    target_frame_time: Option<Duration>,
    next_frame_at: Option<Instant>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        let target_frame_time = config
            .time
            .target_fps
            .filter(|fps| *fps > 0)
            .map(|fps| Duration::from_secs(1) / fps);
        let fixed_clock = FixedClock::new(config.time.fixed_timestep);
        Self {
            config,
            state: EngineState::Created,
            context: EngineContext::default(),
            clock: FrameClock::new(),
            fixed_clock,
            subsystems: Vec::new(),
            initialized: 0,
            window: None,
            auto_paused: false,
            suspended: false,
            target_frame_time,
            next_frame_at: None,
        }
    }

    /// Enregistre un subsystem ; l'ordre d'enregistrement définit l'ordre d'init.
    pub fn add_subsystem(&mut self, subsystem: Box<dyn Subsystem>) -> &mut Self {
        self.subsystems.push(subsystem);
        self
    }

    /// Le profil CPU de la dernière frame complète — l'accès aux
    /// RÉSULTATS pour l'application, avant ou après `run()` (pendant le
    /// run, les subsystems lisent le même service via le contexte).
    pub fn diagnostics(&self) -> &crate::diagnostics::FrameDiagnostics {
        self.context.diagnostics()
    }

    /// La santé synthétique du moteur (`snapshot()`) — l'accès aux
    /// résultats pour l'application, avant ou après `run()`.
    pub fn metrics(&self) -> &crate::metrics::EngineMetrics {
        self.context.metrics()
    }

    /// Démarre le moteur et bloque jusqu'à l'arrêt propre.
    /// En mode fenêtré : doit être appelé depuis le main thread, une seule
    /// fois par processus (les OS ne permettent pas de recréer la boucle
    /// d'événements). En mode headless : aucune contrainte de thread.
    ///
    /// La configuration est validée EN PREMIER : une configuration
    /// invalide échoue ici, avant la boucle d'événements, la fenêtre et
    /// toute initialisation partielle.
    ///
    /// Le modèle d'échec (`docs/architecture/engine-loop.md`) : toute
    /// défaillance FATALE — configuration, initialisation, exécution
    /// (schedule ECS, rendu, escalade `report_fatal`) — provoque un arrêt
    /// ordonné avec nettoyage complet et ressort ici en `Err` précis ; la
    /// PREMIÈRE défaillance est la cause. Une demande d'arrêt normale
    /// (`request_exit`, `frame_limit`) n'est PAS une erreur : `Ok(())`.
    pub fn run(&mut self) -> ChaosResult<()> {
        if self.state != EngineState::Created {
            return Err(ChaosError::Engine(String::from(
                "the engine already ran: one lifecycle per engine, build a new one",
            )));
        }
        self.config.validate()?;
        let mode = if self.config.runtime.headless {
            " headless"
        } else {
            ""
        };
        info!(
            "{} starting{mode} (Chaos Engine {})",
            self.config.app.name,
            env!("CARGO_PKG_VERSION")
        );
        if self.config.runtime.headless {
            self.run_headless();
        } else {
            run_event_loop(self.config.window.clone(), self)?;
        }
        if let Some(fatal) = self.context.take_fatal() {
            return Err(fatal);
        }
        info!("{} stopped cleanly", self.config.app.name);
        Ok(())
    }

    /// La boucle headless : LES MÊMES hooks que la boucle OS (`start` →
    /// `on_update` → `on_shutdown`), pilotés par le moteur lui-même — le
    /// même Engine Core orchestre les deux modes, seul le driver change.
    /// Pas de `on_redraw` : la phase présentation n'existe pas en
    /// headless. La cadence (`time.target_fps`) est tenue par sleep — il
    /// n'y a aucun événement à pomper entre les ticks ; `None` = boucle
    /// libre.
    fn run_headless(&mut self) {
        self.start();
        while !self.context.exit_requested() {
            if let Some(deadline) = self.next_frame_at {
                let now = Instant::now();
                if deadline > now {
                    std::thread::sleep(deadline - now);
                }
            }
            self.on_update();
        }
        self.on_shutdown();
    }

    fn start(&mut self) {
        if self.state != EngineState::Created {
            error!("cannot start the engine: state is {:?}", self.state);
            return;
        }
        if let Err(config_error) = self.config.validate() {
            error!("invalid engine configuration: {config_error}");
            self.context.store_fatal(config_error);
            return;
        }
        // Rien de périmé ne traverse le démarrage : une requête de pause
        // posée avant le start est purgée.
        if self.context.take_pause_request().is_some() {
            debug!("stale pause request discarded at startup");
        }
        if let Err(schedule_error) = self.setup_schedule() {
            error!("engine schedule setup failed: {schedule_error}");
            self.context.store_fatal(schedule_error);
            return;
        }
        if let Err(disable_error) = self.apply_disabled_subsystems() {
            error!("subsystem disabling failed: {disable_error}");
            self.context.store_fatal(disable_error);
            return;
        }
        let skipped = self.apply_headless_filter();
        if let Err(order_error) = self.sort_subsystems() {
            error!("subsystem ordering failed: {order_error}");
            self.context.store_fatal(order_error);
            return;
        }
        for index in 0..self.subsystems.len() {
            let name = self.subsystems[index].name().to_owned();
            debug!("init subsystem '{name}'");
            if let Err(init_error) = self.subsystems[index].init(&mut self.context) {
                error!("subsystem '{name}' failed to init: {init_error}");
                self.context.store_fatal(init_error);
                return;
            }
            self.initialized = index + 1;
        }
        let mut statuses: Vec<SubsystemStatus> = self
            .subsystems
            .iter()
            .map(|subsystem| SubsystemStatus {
                name: subsystem.name().to_owned(),
                state: SubsystemState::Active,
            })
            .collect();
        statuses.extend(self.config.runtime.disabled_subsystems.iter().map(|name| {
            SubsystemStatus {
                name: name.clone(),
                state: SubsystemState::Disabled,
            }
        }));
        statuses.extend(skipped.into_iter().map(|name| SubsystemStatus {
            name,
            state: SubsystemState::SkippedHeadless,
        }));
        self.context.metrics_mut().set_subsystems(statuses);
        self.context
            .diagnostics_mut()
            .set_budget(self.target_frame_time);
        self.clock = FrameClock::new();
        self.state = EngineState::Running;
        info!("engine running ({} subsystem(s))", self.subsystems.len());
    }

    /// Les phases moteur et les systèmes-services : UPDATE (simulation) →
    /// LATE_UPDATE (mises à jour tardives) → POST_UPDATE (les calculs
    /// dérivés — la propagation des transforms, garantie par le moteur).
    fn setup_schedule(&mut self) -> ChaosResult<()> {
        let schedule = self.context.schedule_mut();
        schedule.add_stage(stages::UPDATE)?;
        schedule.add_stage(stages::LATE_UPDATE)?;
        schedule.add_stage(stages::POST_UPDATE)?;
        schedule.add_system(stages::POST_UPDATE, TransformPropagation)?;
        self.context
            .fixed_schedule_mut()
            .add_stage(stages::FIXED_UPDATE)
    }

    /// Retire les subsystems désactivés par configuration AVANT le tri des
    /// dépendances : jamais initialisés, jamais tickés. Un nom qui ne
    /// correspond à aucun subsystem enregistré est refusé — une
    /// désactivation qui ne désactive rien est une erreur de
    /// configuration, jamais un no-op silencieux.
    fn apply_disabled_subsystems(&mut self) -> ChaosResult<()> {
        let disabled = &self.config.runtime.disabled_subsystems;
        for name in disabled {
            if !self
                .subsystems
                .iter()
                .any(|subsystem| subsystem.name() == name)
            {
                return Err(ChaosError::Config(format!(
                    "cannot disable subsystem '{name}': no such subsystem is registered"
                )));
            }
        }
        self.subsystems.retain(|subsystem| {
            let keep = !disabled.iter().any(|name| name == subsystem.name());
            if !keep {
                info!("subsystem '{}' disabled by configuration", subsystem.name());
            }
            keep
        });
        Ok(())
    }

    /// En mode headless, retire les subsystems GRAPHIQUES
    /// (`requires_graphics()`) AVANT le tri : jamais initialisés, jamais
    /// tickés, avec `info!` par retrait. Un subsystem restant qui dépend
    /// d'un retiré échoue au tri avec le refus précis existant
    /// (« depends on 'x' which is not registered ») — la cascade voulue :
    /// dépendre d'un graphique, c'est être graphique.
    fn apply_headless_filter(&mut self) -> Vec<String> {
        if !self.config.runtime.headless {
            return Vec::new();
        }
        let mut skipped = Vec::new();
        self.subsystems.retain(|subsystem| {
            let keep = !subsystem.requires_graphics();
            if !keep {
                info!(
                    "subsystem '{}' skipped in headless mode (requires graphics)",
                    subsystem.name()
                );
                skipped.push(subsystem.name().to_owned());
            }
            keep
        });
        skipped
    }

    /// Trie les subsystems par dépendances déclarées (Kahn STABLE : à
    /// égalité, l'ordre d'enregistrement départage — déterminisme total ;
    /// sans dépendances, le tri est l'identité). Refus explicites : noms
    /// dupliqués, dépendance inconnue, cycle (les participants nommés).
    fn sort_subsystems(&mut self) -> ChaosResult<()> {
        let names: Vec<String> = self
            .subsystems
            .iter()
            .map(|subsystem| subsystem.name().to_owned())
            .collect();
        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name) {
                return Err(ChaosError::Engine(format!(
                    "duplicate subsystem name '{name}': dependency ordering needs unique names"
                )));
            }
        }
        for (index, subsystem) in self.subsystems.iter().enumerate() {
            for dependency in subsystem.dependencies() {
                if !names.iter().any(|name| name == dependency) {
                    return Err(ChaosError::Engine(format!(
                        "subsystem '{}' depends on '{dependency}' which is not registered",
                        names[index]
                    )));
                }
            }
        }
        let mut placed: Vec<usize> = Vec::with_capacity(names.len());
        let mut remaining: Vec<usize> = (0..names.len()).collect();
        while !remaining.is_empty() {
            let next = remaining.iter().position(|&candidate| {
                self.subsystems[candidate]
                    .dependencies()
                    .iter()
                    .all(|dependency| placed.iter().any(|&done| names[done] == *dependency))
            });
            let Some(position) = next else {
                let stuck: Vec<&str> = remaining
                    .iter()
                    .map(|&index| names[index].as_str())
                    .collect();
                return Err(ChaosError::Engine(format!(
                    "subsystem dependency cycle among: {}",
                    stuck.join(", ")
                )));
            };
            placed.push(remaining.remove(position));
        }
        let mut slots: Vec<Option<Box<dyn Subsystem>>> = std::mem::take(&mut self.subsystems)
            .into_iter()
            .map(Some)
            .collect();
        self.subsystems = placed
            .iter()
            .filter_map(|&index| slots[index].take())
            .collect();
        if self.subsystems.len() != names.len() {
            return Err(ChaosError::Engine(String::from(
                "subsystem reordering lost an entry: this is a chaos_engine bug",
            )));
        }
        Ok(())
    }

    /// La politique moteur face aux interruptions (focus, suspension OS) —
    /// appliquée AVANT le dispatch : les subsystems reçoivent ensuite
    /// l'événement, c'est leur signal de purge (chacun possède son état
    /// tenu — le patron `DebugCameraController`). La pause de l'APP est
    /// respectée : `auto_paused` n'est posé que quand le MOTEUR initie la
    /// pause — un retour de focus ou une reprise OS ne relance jamais une
    /// pause demandée par l'application.
    fn apply_interruption_policy(&mut self, event: &Event) {
        match event {
            Event::Window(WindowEvent::Focused(false)) => {
                if self.config.runtime.pause_on_focus_loss && self.state == EngineState::Running {
                    info!("focus lost, pausing the simulation");
                    self.context.request_pause();
                    self.auto_paused = true;
                }
            }
            Event::Window(WindowEvent::Focused(true)) => {
                if self.auto_paused {
                    info!("focus regained, resuming the simulation");
                    self.context.request_resume();
                }
            }
            Event::Window(WindowEvent::Suspended) => {
                self.suspended = true;
                if self.state == EngineState::Running {
                    info!("application suspended, pausing the simulation");
                    self.context.request_pause();
                    self.auto_paused = true;
                }
            }
            Event::Window(WindowEvent::Resumed) => {
                self.suspended = false;
                if self.auto_paused {
                    info!("application resumed, resuming the simulation");
                    self.context.request_resume();
                }
            }
            _ => {}
        }
    }

    /// Applique une requête de pause/reprise à la frontière de frame — le
    /// point déterministe. Hors état pertinent, la requête est écartée
    /// avec trace : un refus explicite, jamais un effet différé surprise.
    /// En headless, TOUTE requête est écartée avec `warn!` : la reprise
    /// arrive par les événements (`on_event`) et ce canal n'existe pas —
    /// la pause y serait un gel définitif (même `frame_limit` est gelé).
    fn apply_pause_request(&mut self) {
        let Some(pause) = self.context.take_pause_request() else {
            return;
        };
        if self.config.runtime.headless {
            warn!(
                "pause request discarded: pause needs an event channel and headless mode has none"
            );
            self.context.metrics_mut().count_warning();
            return;
        }
        match (self.state, pause) {
            (EngineState::Running, true) => {
                info!("engine paused");
                self.state = EngineState::Paused;
                self.context.set_paused(true);
            }
            (EngineState::Paused, false) => {
                info!("engine resumed");
                // L'horloge est RESYNCHRONISÉE : la pause ne produit aucun
                // delta (zéro saut de simulation, zéro rafale de pas
                // fixes) ; sa durée reste créditée au temps réel.
                self.clock.resync();
                self.state = EngineState::Running;
                self.context.set_paused(false);
                self.auto_paused = false;
            }
            (state, request) => {
                debug!("pause request discarded (state {state:?}, requested pause={request})");
            }
        }
    }
}

impl WindowEventHandler for Engine {
    fn on_window_ready(&mut self, window: WindowHandle) {
        let (width, height) = window.inner_size();
        info!(
            "window ready: {width}x{height} (scale factor {})",
            window.scale_factor()
        );
        match Renderer::attach(
            window.clone(),
            RendererConfig {
                width,
                height,
                vsync: self.config.render.vsync,
            },
        ) {
            Ok(mut renderer) => {
                renderer.set_clear_color(self.config.render.clear_color);
                self.context.set_renderer(renderer);
            }
            Err(attach_error) => {
                error!("renderer initialization failed: {attach_error}");
                self.context.store_fatal(attach_error);
                self.window = Some(window);
                return;
            }
        }
        self.subsystems.push(Box::new(RenderSubsystem));
        self.window = Some(window);
        self.start();
    }

    fn on_event(&mut self, event: Event) {
        if self.state == EngineState::Stopped {
            return;
        }
        trace!("event: {event:?}");
        if event == Event::Window(WindowEvent::CloseRequested) {
            info!("close requested by the system");
            self.context.request_exit();
        }
        self.apply_interruption_policy(&event);
        self.context.world_mut().send_message(event);
        for subsystem in &mut self.subsystems[..self.initialized] {
            subsystem.on_event(&event, &mut self.context);
        }
    }

    fn on_update(&mut self) {
        if !matches!(self.state, EngineState::Running | EngineState::Paused)
            || self.context.exit_requested()
        {
            return;
        }
        let now = Instant::now();
        if let Some(next_frame_at) = self.next_frame_at
            && now < next_frame_at
        {
            return;
        }
        self.apply_pause_request();
        if self.state == EngineState::Paused {
            // Simulation gelée : pas de tick, pas de schedule, pas
            // d'updates. Les messages sont balayés (personne ne les
            // consomme — les subsystems les reçoivent par on_event) et le
            // rendu continue : la fenêtre reste vivante.
            self.context.world_mut().clear_messages();
            if let Some(target) = self.target_frame_time {
                self.next_frame_at = Some(now + target);
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        }
        self.clock.set_scale(self.context.time_scale());
        let time = self.clock.tick();
        self.context.set_time(time);
        // Le real_delta qui vient d'être mesuré EST la durée murale de la
        // frame précédente : elle est close avec, la nouvelle s'ouvre.
        let diagnostics = self.context.diagnostics_mut();
        diagnostics.close_frame(time.real_delta);
        diagnostics.begin_frame(time.frame_index);
        if time.frame_index > 1 {
            self.context.metrics_mut().record_frame(time.real_delta);
        }
        let update_start = Instant::now();
        let steps = self.fixed_clock.advance(time.delta);
        if let Err(schedule_error) = self.context.tick_world(time, &mut self.fixed_clock, steps) {
            // La frame est ABANDONNÉE : le monde est en état inconnu,
            // aucun code applicatif ne tourne dessus — l'arrêt ordonné
            // suit à la frontière de frame, le diagnostic ressort de run().
            error!("ecs schedule failed: {schedule_error}");
            self.context.store_fatal(schedule_error);
            return;
        }
        if let Some(renderer) = self.context.renderer_mut() {
            renderer.clear_draws();
        }
        for index in 0..self.initialized {
            let span_start = Instant::now();
            self.subsystems[index].update(&mut self.context);
            let name = self.subsystems[index].name();
            self.context
                .diagnostics_mut()
                .record_subsystem(index, name, span_start.elapsed());
        }
        self.context.world_mut().clear_messages();
        self.context
            .diagnostics_mut()
            .record_update_total(update_start.elapsed());
        let entities = self.context.world().len();
        let active_scenes = self.context.scenes().actives().len();
        let loaded_assets = self.context.assets().registry().loaded_count();
        let tracked_bytes = self.context.assets().cached_bytes();
        let draw_calls = self
            .context
            .renderer()
            .map(|renderer| renderer.draw_count())
            .unwrap_or(0);
        self.context.metrics_mut().sample(
            time.frame_index,
            entities,
            active_scenes,
            loaded_assets,
            draw_calls,
            tracked_bytes,
        );
        if let Some(limit) = self.config.runtime.frame_limit
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
        if !matches!(self.state, EngineState::Running | EngineState::Paused)
            || self.context.exit_requested()
        {
            return;
        }
        // Suspendu par l'OS : les hooks render ne sont même pas appelés
        // (le backend a en plus sa propre garde taille-nulle).
        if self.suspended {
            return;
        }
        let render_start = Instant::now();
        for index in 0..self.initialized {
            let span_start = Instant::now();
            self.subsystems[index].render(&mut self.context);
            let name = self.subsystems[index].name();
            self.context
                .diagnostics_mut()
                .record_render(index, name, span_start.elapsed());
        }
        self.context
            .diagnostics_mut()
            .record_render_total(render_start.elapsed());
    }

    fn frame_deadline(&self) -> Option<Instant> {
        if !matches!(self.state, EngineState::Running | EngineState::Paused)
            || self.context.exit_requested()
        {
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
        if let Err(scene_error) = self.context.shutdown_scenes() {
            // Best-effort : le nettoyage continue ; surfacé par run() s'il
            // est la SEULE défaillance, sinon logué comme conséquence.
            error!("scene cleanup failed during shutdown: {scene_error}");
            self.context.store_fatal(scene_error);
        }
        // Les travaux en attente sont annulés, le contexte redevient
        // vierge (le filet après le déchargement déterministe des
        // scènes), les caches d'assets sont fermés.
        self.context.take_pause_request();
        self.context.reset_world();
        self.context.shutdown_assets();
        let frames = self.context.time().frame_index;
        if frames > 0 {
            info!(
                "diagnostics: {frames} frame(s), {} over budget",
                self.context.diagnostics().overruns()
            );
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
    use std::fs;
    use std::rc::Rc;

    use chaos_core::{ElementState, InputEvent, KeyCode, Time};
    use chaos_ecs::{Resource, System, World};

    use super::*;
    use crate::config::{RuntimeConfig, TimeConfig};

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
        deps: &'static [&'static str],
        graphics: bool,
    }

    impl Probe {
        fn boxed(name: &'static str, journal: &Journal) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: false,
                deps: &[],
                graphics: false,
            })
        }

        fn failing(name: &'static str, journal: &Journal) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: true,
                deps: &[],
                graphics: false,
            })
        }

        fn depending(
            name: &'static str,
            deps: &'static [&'static str],
            journal: &Journal,
        ) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: false,
                deps,
                graphics: false,
            })
        }

        fn graphics(name: &'static str, journal: &Journal) -> Box<Self> {
            Box::new(Self {
                name,
                journal: journal.clone(),
                fail_init: false,
                deps: &[],
                graphics: true,
            })
        }
    }

    impl Subsystem for Probe {
        fn name(&self) -> &str {
            self.name
        }

        fn dependencies(&self) -> &[&str] {
            self.deps
        }

        fn requires_graphics(&self) -> bool {
            self.graphics
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
            time: TimeConfig {
                target_fps: None,
                ..TimeConfig::default()
            },
            ..EngineConfig::default()
        }
    }

    fn headless_config(frame_limit: Option<u64>) -> EngineConfig {
        EngineConfig {
            runtime: RuntimeConfig {
                headless: true,
                frame_limit,
                ..RuntimeConfig::default()
            },
            ..unpaced_config()
        }
    }

    #[derive(Debug, PartialEq)]
    struct Ticks(u32);

    impl Resource for Ticks {}

    struct CountUp;

    impl System for CountUp {
        fn name(&self) -> &str {
            "count_up"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            if let Some(ticks) = world.resource_mut::<Ticks>() {
                ticks.0 += 1;
            }
            Ok(())
        }
    }

    struct FailingSystem;

    impl System for FailingSystem {
        fn name(&self) -> &str {
            "explode"
        }

        fn run(&self, _world: &mut World) -> ChaosResult<()> {
            Err(ChaosError::Ecs(String::from("boom")))
        }
    }

    struct Installer {
        failing: bool,
    }

    impl Subsystem for Installer {
        fn name(&self) -> &str {
            "installer"
        }

        fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
            context.world_mut().insert_resource(Ticks(0));
            if self.failing {
                context
                    .schedule_mut()
                    .add_system(stages::UPDATE, FailingSystem)
            } else {
                context.schedule_mut().add_system(stages::UPDATE, CountUp)
            }
        }
    }

    struct MessageProbe {
        journal: Journal,
    }

    impl Subsystem for MessageProbe {
        fn name(&self) -> &str {
            "message_probe"
        }

        fn update(&mut self, context: &mut EngineContext) {
            let seen = context.world().messages::<Event>().count();
            self.journal.push(format!("messages {seen}"));
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
            runtime: RuntimeConfig {
                frame_limit: Some(3),
                ..RuntimeConfig::default()
            },
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
        assert!(engine.context.fatal().is_some());
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
            time: TimeConfig {
                target_fps: Some(60),
                ..TimeConfig::default()
            },
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

    #[test]
    fn the_world_receives_time_as_a_resource() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        let time = engine.context.time();
        assert_eq!(time.frame_index, 1);
        assert_eq!(engine.context.world().resource::<Time>(), Some(&time));
    }

    #[test]
    fn a_pumped_event_is_a_message_for_exactly_one_update() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(MessageProbe {
            journal: journal.clone(),
        }));
        engine.start();
        engine.on_event(Event::Input(InputEvent::Keyboard {
            key: KeyCode::Escape,
            state: ElementState::Pressed,
            repeat: false,
        }));
        engine.on_update();
        engine.on_update();
        assert_eq!(journal.entries(), vec!["messages 1", "messages 0"]);
    }

    #[test]
    fn a_registered_system_runs_every_update() {
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(Installer { failing: false }));
        engine.start();
        engine.on_update();
        engine.on_update();
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Ticks>(), Some(&Ticks(3)));
    }

    #[test]
    fn a_failing_system_stops_the_engine_cleanly() {
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(Installer { failing: true }));
        engine.start();
        assert!(!engine.exit_requested());
        engine.on_update();
        assert!(engine.exit_requested());
    }

    #[test]
    fn the_engine_shutdown_cleans_the_scenes() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        let (world, scenes) = engine.context.world_and_scenes();
        let id = scenes.create("maps/spawn").unwrap();
        scenes
            .load(world, id, |scene, world| {
                scene.spawn(world)?;
                scene.spawn(world)?;
                Ok(())
            })
            .unwrap();
        scenes.activate(id).unwrap();
        assert_eq!(engine.context.world().len(), 2);
        engine.on_shutdown();
        assert!(engine.context.world().is_empty());
        assert!(engine.context.scenes().is_empty());
        assert_eq!(engine.context.scenes().main(), None);
    }

    #[test]
    fn subsystems_communicate_through_the_world_not_each_other() {
        #[derive(Debug, PartialEq)]
        struct Ping(u32);
        impl chaos_ecs::Message for Ping {}

        struct Producer;
        impl Subsystem for Producer {
            fn name(&self) -> &str {
                "producer"
            }
            fn update(&mut self, context: &mut EngineContext) {
                context.world_mut().send_message(Ping(7));
            }
        }

        struct Consumer {
            journal: Journal,
        }
        impl Subsystem for Consumer {
            fn name(&self) -> &str {
                "consumer"
            }
            fn update(&mut self, context: &mut EngineContext) {
                let sum: u32 = context.world().messages::<Ping>().map(|ping| ping.0).sum();
                self.journal.push(format!("received {sum}"));
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(Producer));
        engine.add_subsystem(Box::new(Consumer {
            journal: journal.clone(),
        }));
        engine.start();
        engine.on_update();
        engine.on_update();
        // Le message émis à la frame N est lu à la frame N (le consommateur
        // passe après) et balayé en fin de frame — jamais cumulé.
        assert_eq!(journal.entries(), vec!["received 7", "received 7"]);
    }

    #[test]
    fn late_system_registration_applies_next_frame() {
        struct LateInstaller {
            installed: bool,
        }
        impl Subsystem for LateInstaller {
            fn name(&self) -> &str {
                "late_installer"
            }
            fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
                context.world_mut().insert_resource(Ticks(0));
                Ok(())
            }
            fn update(&mut self, context: &mut EngineContext) {
                if !self.installed {
                    self.installed = true;
                    let outcome = context.schedule_mut().add_system(stages::UPDATE, CountUp);
                    assert!(outcome.is_ok());
                }
            }
        }

        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(LateInstaller { installed: false }));
        engine.start();
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Ticks>(), Some(&Ticks(0)));
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Ticks>(), Some(&Ticks(1)));
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Ticks>(), Some(&Ticks(2)));
    }

    #[test]
    fn services_are_usable_across_every_hook() {
        struct ServiceProbe {
            journal: Journal,
        }
        impl ServiceProbe {
            fn check(&self, hook: &str, context: &EngineContext) {
                let ok = context.assets().registry().is_empty()
                    && context.scenes().is_empty()
                    && context.world().is_empty();
                self.journal.push(format!("{hook} services ok: {ok}"));
            }
        }
        impl Subsystem for ServiceProbe {
            fn name(&self) -> &str {
                "service_probe"
            }
            fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
                self.check("init", context);
                Ok(())
            }
            fn update(&mut self, context: &mut EngineContext) {
                self.check("update", context);
            }
            fn shutdown(&mut self, context: &mut EngineContext) {
                self.check("shutdown", context);
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(ServiceProbe {
            journal: journal.clone(),
        }));
        engine.start();
        engine.on_update();
        engine.on_shutdown();
        assert_eq!(
            journal.entries(),
            vec![
                "init services ok: true",
                "update services ok: true",
                "shutdown services ok: true",
            ]
        );
    }

    #[test]
    fn valid_dependencies_yield_a_topological_order() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::depending("b", &["c"], &journal));
        engine.add_subsystem(Probe::depending("c", &["a"], &journal));
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_shutdown();
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "init c",
                "init b",
                "shutdown b",
                "shutdown c",
                "shutdown a"
            ]
        );
    }

    #[test]
    fn registration_order_breaks_ties_deterministically() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("b", &journal));
        engine.add_subsystem(Probe::depending("x", &["a"], &journal));
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        assert_eq!(journal.entries(), vec!["init b", "init a", "init x"]);
    }

    #[test]
    fn a_dependency_cycle_is_refused_cleanly() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::depending("a", &["b"], &journal));
        engine.add_subsystem(Probe::depending("b", &["a"], &journal));
        engine.start();
        engine.on_update();
        assert!(journal.entries().is_empty());
        assert!(engine.exit_requested());
        assert!(
            engine
                .context
                .fatal()
                .is_some_and(|error| error.to_string().contains("cycle"))
        );
    }

    #[test]
    fn a_missing_dependency_is_refused_cleanly() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::depending("x", &["ghost"], &journal));
        engine.start();
        assert!(journal.entries().is_empty());
        let error = engine.context.fatal().unwrap().to_string();
        assert!(error.contains("'x'"));
        assert!(error.contains("'ghost'"));
        assert!(error.contains("not registered"));
    }

    #[test]
    fn duplicate_subsystem_names_are_refused() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        assert!(journal.entries().is_empty());
        assert!(
            engine
                .context
                .fatal()
                .is_some_and(|error| error.to_string().contains("duplicate subsystem name 'a'"))
        );
    }

    #[test]
    fn an_invalid_config_fails_before_any_partial_initialization() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            time: TimeConfig {
                target_fps: Some(0),
                ..TimeConfig::default()
            },
            ..EngineConfig::default()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        assert!(engine.exit_requested());
        let error = engine.context.fatal().unwrap().to_string();
        assert!(error.starts_with("configuration error: "));
        assert!(error.contains("target_fps"));
        engine.on_update();
        engine.on_shutdown();
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn subsystems_disabled_by_configuration_never_run() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            runtime: RuntimeConfig {
                disabled_subsystems: vec![String::from("a")],
                ..RuntimeConfig::default()
            },
            ..unpaced_config()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.add_subsystem(Probe::boxed("b", &journal));
        engine.start();
        engine.on_update();
        engine.on_redraw();
        engine.on_shutdown();
        assert_eq!(
            journal.entries(),
            vec!["init b", "update b 1", "render b", "shutdown b"]
        );
    }

    #[test]
    fn disabling_an_unknown_subsystem_is_refused() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            runtime: RuntimeConfig {
                disabled_subsystems: vec![String::from("ghost")],
                ..RuntimeConfig::default()
            },
            ..unpaced_config()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        assert!(journal.entries().is_empty());
        let error = engine.context.fatal().unwrap().to_string();
        assert!(error.contains("'ghost'"));
        assert!(error.contains("no such subsystem is registered"));
    }

    #[test]
    fn a_headless_run_executes_the_configured_ticks_then_stops_cleanly() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "update a 1",
                "update a 2",
                "update a 3",
                "update a 4",
                "update a 5",
                "shutdown a"
            ]
        );
    }

    #[test]
    fn graphics_subsystems_are_skipped_in_headless_mode() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(1)));
        engine.add_subsystem(Probe::graphics("gpu", &journal));
        engine.add_subsystem(Probe::boxed("sim", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(
            journal.entries(),
            vec!["init sim", "update sim 1", "shutdown sim"]
        );
    }

    #[test]
    fn graphics_subsystems_are_kept_outside_headless_mode() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::graphics("gpu", &journal));
        engine.start();
        engine.on_update();
        assert_eq!(journal.entries(), vec!["init gpu", "update gpu 1"]);
    }

    #[test]
    fn depending_on_a_skipped_graphics_subsystem_is_refused_precisely() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(1)));
        engine.add_subsystem(Probe::graphics("gpu", &journal));
        engine.add_subsystem(Probe::depending("hud", &["gpu"], &journal));
        let error = engine.run().unwrap_err().to_string();
        assert!(error.contains("'hud'"));
        assert!(error.contains("'gpu'"));
        assert!(error.contains("not registered"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn an_unbounded_headless_run_stops_when_a_subsystem_requests_exit() {
        struct QuitAt {
            tick: u64,
            journal: Journal,
        }
        impl Subsystem for QuitAt {
            fn name(&self) -> &str {
                "quit_at"
            }
            fn update(&mut self, context: &mut EngineContext) {
                let frame = context.time().frame_index;
                self.journal.push(format!("tick {frame}"));
                if frame >= self.tick {
                    context.request_exit();
                }
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(QuitAt {
            tick: 3,
            journal: journal.clone(),
        }));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(journal.entries(), vec!["tick 1", "tick 2", "tick 3"]);
    }

    #[test]
    fn a_pause_request_is_discarded_in_headless_mode() {
        struct PauseAsker;
        impl Subsystem for PauseAsker {
            fn name(&self) -> &str {
                "pause_asker"
            }
            fn update(&mut self, context: &mut EngineContext) {
                context.request_pause();
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(3)));
        engine.add_subsystem(Box::new(PauseAsker));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert!(!engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "update a 1",
                "update a 2",
                "update a 3",
                "shutdown a"
            ]
        );
    }

    #[test]
    fn a_paced_headless_run_terminates() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            time: TimeConfig {
                target_fps: Some(1000),
                ..TimeConfig::default()
            },
            runtime: RuntimeConfig {
                headless: true,
                frame_limit: Some(3),
                ..RuntimeConfig::default()
            },
            ..EngineConfig::default()
        });
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "update a 1",
                "update a 2",
                "update a 3",
                "shutdown a"
            ]
        );
    }

    #[test]
    fn a_failing_init_surfaces_through_a_headless_run() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Probe::boxed("base", &journal));
        engine.add_subsystem(Probe::failing("boom", &journal));
        let error = engine.run().unwrap_err().to_string();
        assert!(error.contains("boom init failed"));
        assert_eq!(
            journal.entries(),
            vec!["init base", "init boom", "shutdown base"]
        );
    }

    #[test]
    fn a_fatal_runtime_failure_surfaces_through_run() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Box::new(Installer { failing: true }));
        engine.add_subsystem(Probe::boxed("a", &journal));
        let error = engine.run().unwrap_err().to_string();
        assert!(error.contains("boom"));
        assert!(journal.entries().contains(&String::from("shutdown a")));
    }

    #[test]
    fn a_fatal_ecs_failure_abandons_the_frame() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Box::new(Installer { failing: true }));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert!(engine.run().is_err());
        // La frame de l'échec est abandonnée : aucun update ne tourne sur
        // un monde en état inconnu — init puis shutdown, rien entre.
        assert_eq!(journal.entries(), vec!["init a", "shutdown a"]);
    }

    #[test]
    fn a_subsystem_escalates_a_fatal_with_its_diagnostic() {
        struct Escalator;
        impl Subsystem for Escalator {
            fn name(&self) -> &str {
                "escalator"
            }
            fn update(&mut self, context: &mut EngineContext) {
                if context.time().frame_index == 2 {
                    context.report_fatal(ChaosError::Engine(String::from(
                        "escalator cannot continue",
                    )));
                }
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(Escalator));
        engine.add_subsystem(Probe::boxed("a", &journal));
        let error = engine.run().unwrap_err().to_string();
        assert_eq!(error, "engine error: escalator cannot continue");
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2", "shutdown a"]
        );
    }

    #[test]
    fn the_first_fatal_wins_the_diagnostic() {
        struct Fail(&'static str);
        impl Subsystem for Fail {
            fn name(&self) -> &str {
                self.0
            }
            fn update(&mut self, context: &mut EngineContext) {
                context.report_fatal(ChaosError::Engine(format!("{} failed", self.0)));
            }
        }

        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(Fail("first")));
        engine.add_subsystem(Box::new(Fail("second")));
        let error = engine.run().unwrap_err().to_string();
        assert_eq!(error, "engine error: first failed");
    }

    #[test]
    fn a_recoverable_failure_handled_locally_never_stops_the_engine() {
        use chaos_assets::{AssetKind, AssetSource};

        struct Recoverer {
            journal: Journal,
            path: std::path::PathBuf,
        }
        impl Subsystem for Recoverer {
            fn name(&self) -> &str {
                "recoverer"
            }
            fn update(&mut self, context: &mut EngineContext) {
                if context.time().frame_index != 1 {
                    return;
                }
                let declared = context.assets_mut().declare(
                    "scenes/corrupt",
                    AssetKind::Scene,
                    AssetSource::File(self.path.clone()),
                );
                let outcome = declared
                    .and_then(|asset| crate::scenes::load_scene(context.assets_mut(), asset));
                match outcome {
                    Ok(_) => self.journal.push("loaded"),
                    Err(_) => self.journal.push("recovered"),
                }
            }
        }

        let path =
            std::env::temp_dir().join(format!("chaos_engine_recover_{}.cscn", std::process::id()));
        fs::write(&path, b"definitely not a scene").unwrap();
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(3)));
        engine.add_subsystem(Box::new(Recoverer {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert!(journal.entries().contains(&String::from("recovered")));
        assert!(journal.entries().contains(&String::from("update a 3")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_normal_exit_request_is_not_an_error() {
        struct Quitter;
        impl Subsystem for Quitter {
            fn name(&self) -> &str {
                "quitter"
            }
            fn update(&mut self, context: &mut EngineContext) {
                if context.time().frame_index == 2 {
                    context.request_exit();
                }
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(Quitter));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2", "shutdown a"]
        );
    }

    #[test]
    fn the_snapshot_reports_coherent_phases_and_subsystems() {
        struct Sleeper;
        impl Subsystem for Sleeper {
            fn name(&self) -> &str {
                "sleeper"
            }
            fn update(&mut self, _context: &mut EngineContext) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        let mut engine = Engine::new(fixed_config(Duration::from_nanos(1)));
        engine.add_subsystem(Box::new(FixedInstaller));
        engine.add_subsystem(Box::new(OrderProbe));
        engine.add_subsystem(Box::new(Sleeper));
        engine.start();
        let_real_time_pass();
        engine.on_update();
        let_real_time_pass();
        engine.on_update();
        let frame = engine.context.diagnostics().last_frame();
        assert_eq!(frame.frame_index, 1);
        assert_eq!(frame.fixed_steps, 5);
        let stage_names: Vec<&str> = frame.stages.iter().map(|span| span.name.as_str()).collect();
        assert_eq!(stage_names, vec!["update", "late_update", "post_update"]);
        let subsystem_names: Vec<&str> = frame
            .subsystems
            .iter()
            .map(|span| span.name.as_str())
            .collect();
        assert_eq!(
            subsystem_names,
            vec!["fixed_installer", "order_probe", "sleeper"]
        );
        assert!(frame.subsystems[2].duration >= Duration::from_millis(2));
        let parts = frame.fixed
            + frame
                .stages
                .iter()
                .map(|span| span.duration)
                .sum::<Duration>()
            + frame
                .subsystems
                .iter()
                .map(|span| span.duration)
                .sum::<Duration>();
        assert!(frame.update >= parts);
        assert!(frame.total >= frame.update);
        assert!(frame.budget.is_none());
        assert!(frame.renders.is_empty());
    }

    #[test]
    fn budget_overruns_count_work_not_pacing() {
        struct Heavy;
        impl Subsystem for Heavy {
            fn name(&self) -> &str {
                "heavy"
            }
            fn update(&mut self, _context: &mut EngineContext) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let mut engine = Engine::new(EngineConfig {
            time: TimeConfig {
                target_fps: Some(1000),
                ..TimeConfig::default()
            },
            runtime: RuntimeConfig {
                headless: true,
                frame_limit: Some(3),
                ..RuntimeConfig::default()
            },
            ..EngineConfig::default()
        });
        engine.add_subsystem(Box::new(Heavy));
        assert_eq!(engine.run(), Ok(()));
        let diagnostics = engine.context.diagnostics();
        assert_eq!(diagnostics.overruns(), 2);
        let frame = diagnostics.last_frame();
        assert_eq!(frame.frame_index, 2);
        assert_eq!(frame.budget, Some(Duration::from_millis(1)));
        assert!(frame.over_budget);
        assert!(frame.work() >= Duration::from_millis(5));
    }

    #[test]
    fn an_unpaced_run_has_no_budget() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(3)));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        let diagnostics = engine.context.diagnostics();
        assert_eq!(diagnostics.overruns(), 0);
        assert_eq!(diagnostics.last_frame().budget, None);
        assert!(!diagnostics.last_frame().over_budget);
    }

    #[test]
    fn the_fixed_frequency_is_reported() {
        struct FixedSleeper;
        impl System for FixedSleeper {
            fn name(&self) -> &str {
                "fixed_sleeper"
            }
            fn run(&self, _world: &mut World) -> ChaosResult<()> {
                std::thread::sleep(Duration::from_micros(200));
                Ok(())
            }
        }
        struct FixedSleepInstaller;
        impl Subsystem for FixedSleepInstaller {
            fn name(&self) -> &str {
                "fixed_sleep_installer"
            }
            fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
                context
                    .fixed_schedule_mut()
                    .add_system(stages::FIXED_UPDATE, FixedSleeper)
            }
        }

        let mut engine = Engine::new(fixed_config(Duration::from_nanos(1)));
        engine.add_subsystem(Box::new(FixedSleepInstaller));
        engine.start();
        let_real_time_pass();
        engine.on_update();
        let_real_time_pass();
        engine.on_update();
        let frame = engine.context.diagnostics().last_frame();
        assert_eq!(frame.frame_index, 1);
        assert_eq!(frame.fixed_steps, 5);
        assert!(frame.fixed >= Duration::from_micros(1000));
    }

    #[test]
    fn render_spans_are_recorded() {
        struct RenderSleeper;
        impl Subsystem for RenderSleeper {
            fn name(&self) -> &str {
                "render_sleeper"
            }
            fn render(&mut self, _context: &mut EngineContext) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(RenderSleeper));
        engine.start();
        engine.on_update();
        engine.on_redraw();
        let_real_time_pass();
        engine.on_update();
        let frame = engine.context.diagnostics().last_frame();
        assert_eq!(frame.frame_index, 1);
        assert_eq!(frame.renders[0].name, "render_sleeper");
        assert!(frame.renders[0].duration >= Duration::from_millis(2));
        assert!(frame.render >= Duration::from_millis(2));
    }

    #[test]
    fn the_snapshot_is_always_a_complete_frame() {
        use crate::diagnostics::FrameProfile;

        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        assert_eq!(
            engine.context.diagnostics().last_frame(),
            &FrameProfile::default()
        );
        engine.on_update();
        assert_eq!(engine.context.diagnostics().last_frame().frame_index, 1);
        engine.on_update();
        assert_eq!(engine.context.diagnostics().last_frame().frame_index, 2);
    }

    #[test]
    fn a_paused_frame_does_not_pollute_the_profile() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        engine.on_update();
        assert_eq!(engine.context.diagnostics().last_frame().frame_index, 1);
        engine.context.request_pause();
        engine.on_update();
        engine.on_update();
        assert_eq!(engine.context.diagnostics().last_frame().frame_index, 1);
        engine.context.request_resume();
        engine.on_update();
        assert_eq!(engine.context.diagnostics().last_frame().frame_index, 2);
    }

    #[test]
    fn an_application_reads_a_coherent_snapshot_during_execution() {
        use chaos_assets::{AssetKind, AssetSource};
        use chaos_core::Transform;
        use chaos_core::math::Vec3;
        use chaos_scene::{EntityData, FORMAT_VERSION, SceneData};

        use crate::metrics::MetricsSnapshot;

        struct App {
            path: std::path::PathBuf,
        }
        impl Subsystem for App {
            fn name(&self) -> &str {
                "app"
            }
            fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
                let asset = context.assets_mut().declare(
                    "scenes/health",
                    AssetKind::Scene,
                    AssetSource::File(self.path.clone()),
                )?;
                let data = crate::scenes::load_scene(context.assets_mut(), asset)?;
                let (world, scenes) = context.world_and_scenes();
                let id = scenes.create(&data.name)?;
                scenes.load(world, id, |scene, world| data.apply(scene, world))?;
                scenes.activate(id)
            }
        }

        struct Reader {
            seen: Rc<RefCell<Option<MetricsSnapshot>>>,
        }
        impl Subsystem for Reader {
            fn name(&self) -> &str {
                "reader"
            }
            fn update(&mut self, context: &mut EngineContext) {
                if context.time().frame_index == 3 {
                    *self.seen.borrow_mut() = Some(context.metrics().snapshot());
                }
            }
        }

        let path =
            std::env::temp_dir().join(format!("chaos_engine_health_{}.cscn", std::process::id()));
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("scenes/health"),
            entities: vec![
                EntityData {
                    transform: Some(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0))),
                    mesh: None,
                    parent: None,
                },
                EntityData {
                    transform: Some(Transform::from_translation(Vec3::new(2.0, 0.0, 0.0))),
                    mesh: None,
                    parent: Some(0),
                },
            ],
        };
        crate::scenes::save_scene(&path, &data).unwrap();

        let seen: Rc<RefCell<Option<MetricsSnapshot>>> = Rc::default();
        let mut engine = Engine::new(headless_config(Some(4)));
        engine.add_subsystem(Box::new(App { path: path.clone() }));
        engine.add_subsystem(Box::new(Reader { seen: seen.clone() }));
        assert_eq!(engine.run(), Ok(()));

        let snapshot = seen.borrow().clone().expect("un snapshot lu au tick 3");
        assert_eq!(snapshot.frame_index, 2);
        assert_eq!(snapshot.entities, 2);
        assert_eq!(snapshot.active_scenes, 1);
        assert_eq!(snapshot.loaded_assets, 1);
        assert_eq!(snapshot.draw_calls, 0);
        assert!(snapshot.tracked_bytes.is_some_and(|bytes| bytes > 0));
        assert!(snapshot.fps > 0.0);
        assert_eq!(
            snapshot.subsystems,
            vec![
                SubsystemStatus {
                    name: String::from("app"),
                    state: SubsystemState::Active,
                },
                SubsystemStatus {
                    name: String::from("reader"),
                    state: SubsystemState::Active,
                },
            ]
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn frame_times_come_from_the_sliding_window() {
        struct Sleepy;
        impl Subsystem for Sleepy {
            fn name(&self) -> &str {
                "sleepy"
            }
            fn update(&mut self, _context: &mut EngineContext) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let mut engine = Engine::new(headless_config(Some(4)));
        engine.add_subsystem(Box::new(Sleepy));
        assert_eq!(engine.run(), Ok(()));
        let snapshot = engine.context.metrics().snapshot();
        assert!(snapshot.frame_time_min >= Duration::from_millis(5));
        assert!(snapshot.frame_time_min <= snapshot.frame_time_avg);
        assert!(snapshot.frame_time_avg <= snapshot.frame_time_max);
        let expected_fps = 1.0 / snapshot.frame_time_avg.as_secs_f32();
        assert!((snapshot.fps - expected_fps).abs() / expected_fps < 0.05);
    }

    #[test]
    fn engine_errors_and_warnings_are_counted() {
        struct Trouble;
        impl Subsystem for Trouble {
            fn name(&self) -> &str {
                "trouble"
            }
            fn update(&mut self, context: &mut EngineContext) {
                match context.time().frame_index {
                    1 => context.request_pause(),
                    2 => context.report_fatal(ChaosError::Engine(String::from("trouble struck"))),
                    _ => {}
                }
            }
        }

        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(Trouble));
        assert!(engine.run().is_err());
        let snapshot = engine.context.metrics().snapshot();
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.warnings, 1);
    }

    #[test]
    fn subsystem_statuses_reflect_the_startup_decisions() {
        let journal = Journal::default();
        let mut engine = Engine::new(EngineConfig {
            runtime: RuntimeConfig {
                headless: true,
                frame_limit: Some(1),
                disabled_subsystems: vec![String::from("optional")],
                ..RuntimeConfig::default()
            },
            ..unpaced_config()
        });
        engine.add_subsystem(Probe::boxed("core", &journal));
        engine.add_subsystem(Probe::boxed("optional", &journal));
        engine.add_subsystem(Probe::graphics("gpu", &journal));
        assert_eq!(engine.run(), Ok(()));
        let snapshot = engine.context.metrics().snapshot();
        assert_eq!(
            snapshot.subsystems,
            vec![
                SubsystemStatus {
                    name: String::from("core"),
                    state: SubsystemState::Active,
                },
                SubsystemStatus {
                    name: String::from("optional"),
                    state: SubsystemState::Disabled,
                },
                SubsystemStatus {
                    name: String::from("gpu"),
                    state: SubsystemState::SkippedHeadless,
                },
            ]
        );
    }

    #[test]
    fn gauges_sample_zero_states_honestly() {
        let mut engine = Engine::new(headless_config(Some(2)));
        assert_eq!(engine.run(), Ok(()));
        let snapshot = engine.context.metrics().snapshot();
        assert_eq!(snapshot.entities, 0);
        assert_eq!(snapshot.active_scenes, 0);
        assert_eq!(snapshot.loaded_assets, 0);
        assert_eq!(snapshot.draw_calls, 0);
        assert_eq!(snapshot.tracked_bytes, Some(0));
        assert_eq!(snapshot.frame_index, 2);
    }

    fn focus_pause_config() -> EngineConfig {
        EngineConfig {
            runtime: RuntimeConfig {
                pause_on_focus_loss: true,
                ..RuntimeConfig::default()
            },
            ..unpaced_config()
        }
    }

    #[test]
    fn focus_loss_pauses_when_the_policy_asks() {
        let journal = Journal::default();
        let mut engine = Engine::new(focus_pause_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_update();
        engine.on_update();
        assert!(engine.context.paused());
        assert_eq!(journal.entries(), vec!["init a", "update a 1", "event a"]);
    }

    #[test]
    fn focus_return_resumes_only_an_auto_pause() {
        let journal = Journal::default();
        let mut engine = Engine::new(focus_pause_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(true)));
        engine.on_update();
        assert!(!engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "event a", "event a", "update a 2"]
        );
    }

    #[test]
    fn an_app_pause_survives_focus_changes() {
        let journal = Journal::default();
        let mut engine = Engine::new(focus_pause_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.context.request_pause();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_event(Event::Window(WindowEvent::Focused(true)));
        engine.on_update();
        engine.on_update();
        assert!(engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "event a", "event a"]
        );
    }

    #[test]
    fn focus_loss_without_the_policy_changes_nothing() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_update();
        assert!(!engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "event a", "update a 2"]
        );
    }

    #[test]
    fn suspension_pauses_the_simulation_and_gates_rendering() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_redraw();
        engine.on_event(Event::Window(WindowEvent::Suspended));
        engine.on_update();
        engine.on_redraw();
        engine.on_event(Event::Window(WindowEvent::Resumed));
        engine.on_update();
        engine.on_redraw();
        assert!(!engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "update a 1",
                "render a",
                "event a",
                "event a",
                "update a 2",
                "render a"
            ]
        );
    }

    #[test]
    fn suspension_while_app_paused_respects_the_app_intent() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.context.request_pause();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Suspended));
        engine.on_event(Event::Window(WindowEvent::Resumed));
        engine.on_update();
        engine.on_update();
        assert!(engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "event a", "event a"]
        );
    }

    #[test]
    fn a_long_interruption_never_yields_a_giant_delta() {
        use chaos_core::FixedTime;

        let mut engine = Engine::new(focus_pause_config());
        engine.start();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_update();
        std::thread::sleep(Duration::from_millis(300));
        engine.on_event(Event::Window(WindowEvent::Focused(true)));
        engine.on_update();
        let time = engine.context.time();
        assert_eq!(time.frame_index, 2);
        assert!(time.delta < Duration::from_millis(50));
        assert!(time.elapsed < Duration::from_millis(50));
        assert!(time.real_elapsed >= Duration::from_millis(300));
        let steps = engine
            .context
            .world()
            .resource::<FixedTime>()
            .map(|fixed| fixed.step_index)
            .unwrap_or_default();
        assert!(steps <= u64::from(chaos_core::FixedClock::DEFAULT_MAX_STEPS) * 2);
    }

    #[test]
    fn no_phantom_input_crosses_an_interruption() {
        struct KeyProbe {
            journal: Journal,
        }
        impl Subsystem for KeyProbe {
            fn name(&self) -> &str {
                "key_probe"
            }
            fn update(&mut self, context: &mut EngineContext) {
                let keys = context
                    .world()
                    .messages::<Event>()
                    .filter(|event| matches!(event, Event::Input(InputEvent::Keyboard { .. })))
                    .count();
                self.journal.push(format!("keys {keys}"));
            }
        }

        let journal = Journal::default();
        let mut engine = Engine::new(focus_pause_config());
        engine.add_subsystem(Box::new(KeyProbe {
            journal: journal.clone(),
        }));
        engine.start();
        engine.on_update();
        engine.on_event(Event::Input(InputEvent::Keyboard {
            key: KeyCode::W,
            state: ElementState::Pressed,
            repeat: false,
        }));
        engine.on_event(Event::Window(WindowEvent::Focused(false)));
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::Focused(true)));
        engine.on_update();
        assert_eq!(journal.entries(), vec!["keys 0", "keys 0"]);
    }

    struct Content {
        journal: Journal,
        path: std::path::PathBuf,
    }

    impl Content {
        fn scene_file(test: &str) -> std::path::PathBuf {
            use chaos_core::Transform;
            use chaos_core::math::Vec3;
            use chaos_scene::{EntityData, FORMAT_VERSION, SceneData};

            let path = std::env::temp_dir().join(format!(
                "chaos_engine_stop_{test}_{}.cscn",
                std::process::id()
            ));
            let data = SceneData {
                version: FORMAT_VERSION,
                name: String::from("scenes/stop"),
                entities: vec![
                    EntityData {
                        transform: Some(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0))),
                        mesh: None,
                        parent: None,
                    },
                    EntityData {
                        transform: Some(Transform::from_translation(Vec3::new(2.0, 0.0, 0.0))),
                        mesh: None,
                        parent: Some(0),
                    },
                ],
            };
            crate::scenes::save_scene(&path, &data).unwrap();
            path
        }
    }

    impl Subsystem for Content {
        fn name(&self) -> &str {
            "content"
        }

        fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
            use chaos_assets::{AssetKind, AssetSource};

            let asset = context.assets_mut().declare(
                "scenes/stop",
                AssetKind::Scene,
                AssetSource::File(self.path.clone()),
            )?;
            let data = crate::scenes::load_scene(context.assets_mut(), asset)?;
            let (world, scenes) = context.world_and_scenes();
            let id = scenes.create(&data.name)?;
            scenes.load(world, id, |scene, world| data.apply(scene, world))?;
            scenes.activate(id)?;
            context.world_mut().spawn()?;
            self.journal.push("init content");
            Ok(())
        }

        fn shutdown(&mut self, _context: &mut EngineContext) {
            self.journal.push("shutdown content");
        }
    }

    fn assert_clean_stop(engine: &Engine) {
        assert!(engine.context.world().is_empty());
        assert_eq!(engine.context.world().messages::<Event>().count(), 0);
        assert!(engine.context.scenes().is_empty());
        assert_eq!(engine.context.scenes().main(), None);
        assert_eq!(engine.context.assets().registry().loaded_count(), 0);
        assert_eq!(engine.context.assets().cached_bytes(), 0);
        assert!(engine.context.renderer().is_none());
    }

    #[test]
    fn a_thousand_pause_resume_cycles_stay_coherent() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        for _ in 0..1000 {
            engine.on_update();
            engine.context.request_pause();
            engine.on_update();
            engine.context.request_resume();
        }
        engine.on_update();
        let time = engine.context.time();
        assert_eq!(time.frame_index, 1001);
        assert!(!engine.context.paused());
        assert_eq!(journal.entries().len(), 1 + 1001);
        assert!(time.elapsed < Duration::from_secs(1));
    }

    #[test]
    fn an_application_reads_results_after_the_run() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(4)));
        engine.add_subsystem(Probe::boxed("app", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_eq!(engine.diagnostics().last_frame().frame_index, 3);
        assert_eq!(engine.diagnostics().overruns(), 0);
        let snapshot = engine.metrics().snapshot();
        assert_eq!(snapshot.frame_index, 4);
        assert_eq!(snapshot.subsystems.len(), 1);
        assert_eq!(snapshot.subsystems[0].name, "app");
        assert_eq!(snapshot.errors, 0);
    }

    #[test]
    fn run_refuses_a_second_lifecycle() {
        let journal = Journal::default();
        let mut engine = Engine::new(headless_config(Some(2)));
        engine.add_subsystem(Probe::boxed("a", &journal));
        assert_eq!(engine.run(), Ok(()));
        let error = engine.run().unwrap_err().to_string();
        assert!(error.contains("build a new one"));
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2", "shutdown a"]
        );
    }

    #[test]
    fn a_frame_limited_run_stops_clean() {
        let journal = Journal::default();
        let path = Content::scene_file("frame_limited");
        let mut engine = Engine::new(headless_config(Some(3)));
        engine.add_subsystem(Box::new(Content {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.add_subsystem(Probe::boxed("b", &journal));
        assert_eq!(engine.run(), Ok(()));
        assert_clean_stop(&engine);
        let entries = journal.entries();
        assert_eq!(entries.first().map(String::as_str), Some("init content"));
        assert_eq!(entries.last().map(String::as_str), Some("shutdown content"));
        assert!(entries.contains(&String::from("shutdown b")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_subsystem_requested_stop_is_clean() {
        struct Quitter;
        impl Subsystem for Quitter {
            fn name(&self) -> &str {
                "quitter"
            }
            fn update(&mut self, context: &mut EngineContext) {
                if context.time().frame_index == 2 {
                    context.request_exit();
                }
            }
        }

        let journal = Journal::default();
        let path = Content::scene_file("subsystem_stop");
        let mut engine = Engine::new(headless_config(None));
        engine.add_subsystem(Box::new(Content {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.add_subsystem(Box::new(Quitter));
        assert_eq!(engine.run(), Ok(()));
        assert_clean_stop(&engine);
        assert!(
            journal
                .entries()
                .contains(&String::from("shutdown content"))
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_system_close_request_stops_clean() {
        let journal = Journal::default();
        let path = Content::scene_file("close_request");
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(Content {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.start();
        engine.on_update();
        engine.on_event(Event::Window(WindowEvent::CloseRequested));
        assert!(engine.exit_requested());
        engine.on_shutdown();
        assert_clean_stop(&engine);
        assert!(
            journal
                .entries()
                .contains(&String::from("shutdown content"))
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_fatal_error_still_releases_everything() {
        let journal = Journal::default();
        let path = Content::scene_file("fatal");
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Box::new(Content {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.add_subsystem(Box::new(Installer { failing: true }));
        let error = engine.run().unwrap_err().to_string();
        assert!(error.contains("boom"));
        assert_clean_stop(&engine);
        assert!(
            journal
                .entries()
                .contains(&String::from("shutdown content"))
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_partial_init_failure_releases_what_was_started() {
        let journal = Journal::default();
        let path = Content::scene_file("partial_init");
        let mut engine = Engine::new(headless_config(Some(5)));
        engine.add_subsystem(Box::new(Content {
            journal: journal.clone(),
            path: path.clone(),
        }));
        engine.add_subsystem(Probe::failing("boom", &journal));
        engine.add_subsystem(Probe::boxed("jamais", &journal));
        assert!(engine.run().is_err());
        assert_clean_stop(&engine);
        let entries = journal.entries();
        assert!(entries.contains(&String::from("shutdown content")));
        assert!(!entries.contains(&String::from("init jamais")));
        assert!(!entries.contains(&String::from("shutdown jamais")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn pending_work_is_cancelled_at_shutdown() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        engine.on_event(Event::Input(InputEvent::CursorEntered));
        engine.on_event(Event::Input(InputEvent::CursorLeft));
        engine.context.request_pause();
        engine.on_shutdown();
        assert_clean_stop(&engine);
        assert!(engine.context.take_pause_request().is_none());
        assert!(!engine.context.paused());
    }

    #[test]
    fn a_stopped_engine_stays_stopped() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_shutdown();
        engine.on_shutdown();
        engine.start();
        engine.on_update();
        engine.on_redraw();
        engine.on_event(Event::Input(InputEvent::CursorEntered));
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "shutdown a"]
        );
        assert_clean_stop(&engine);
    }

    #[test]
    fn a_failed_init_in_sorted_order_cleans_up_in_reverse_sorted_order() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        // Enregistrés : [boom(dep base), base, jamais(dep boom)] —
        // ordre trié : [base, boom, jamais]. boom échoue : seul base
        // (initialisé AVANT lui dans l'ordre trié) est nettoyé.
        engine.add_subsystem(Box::new(Probe {
            name: "boom",
            journal: journal.clone(),
            fail_init: true,
            deps: &["base"],
            graphics: false,
        }));
        engine.add_subsystem(Probe::boxed("base", &journal));
        engine.add_subsystem(Probe::depending("jamais", &["boom"], &journal));
        engine.start();
        engine.on_shutdown();
        assert_eq!(
            journal.entries(),
            vec!["init base", "init boom", "shutdown base"]
        );
    }

    #[test]
    fn every_hook_follows_the_sorted_order() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::depending("second", &["first"], &journal));
        engine.add_subsystem(Probe::boxed("first", &journal));
        engine.start();
        engine.on_event(Event::Input(InputEvent::CursorEntered));
        engine.on_update();
        engine.on_redraw();
        assert_eq!(
            journal.entries(),
            vec![
                "init first",
                "init second",
                "event first",
                "event second",
                "update first 1",
                "update second 1",
                "render first",
                "render second",
            ]
        );
    }

    #[derive(Debug, PartialEq, Default)]
    struct Order(Vec<&'static str>);

    impl chaos_ecs::Resource for Order {}

    struct Mark(&'static str);

    impl System for Mark {
        fn name(&self) -> &str {
            self.0
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            if let Some(order) = world.resource_mut::<Order>() {
                order.0.push(self.0);
            }
            Ok(())
        }
    }

    struct OrderProbe;

    impl Subsystem for OrderProbe {
        fn name(&self) -> &str {
            "order_probe"
        }

        fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
            context.world_mut().insert_resource(Order::default());
            let schedule = context.schedule_mut();
            schedule.add_system(stages::UPDATE, Mark("update"))?;
            schedule.add_system(stages::LATE_UPDATE, Mark("late_update"))?;
            schedule.add_system(stages::POST_UPDATE, Mark("post_update"))
        }

        fn update(&mut self, context: &mut EngineContext) {
            if let Some(order) = context.world_mut().resource_mut::<Order>() {
                order.0.push("subsystem");
            }
        }
    }

    #[test]
    fn the_frame_follows_the_documented_execution_order() {
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(OrderProbe));
        engine.start();
        engine.on_update();
        engine.on_update();
        let order = engine.context.world().resource::<Order>().unwrap();
        assert_eq!(
            order.0,
            vec![
                "update",
                "late_update",
                "post_update",
                "subsystem",
                "update",
                "late_update",
                "post_update",
                "subsystem",
            ]
        );
    }

    #[test]
    fn subsystems_run_in_registration_order_in_every_hook() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.add_subsystem(Probe::boxed("b", &journal));
        engine.start();
        engine.on_update();
        engine.on_redraw();
        engine.on_update();
        assert_eq!(
            journal.entries(),
            vec![
                "init a",
                "init b",
                "update a 1",
                "update b 1",
                "render a",
                "render b",
                "update a 2",
                "update b 2",
            ]
        );
    }

    #[test]
    fn the_execution_order_is_identical_across_runs() {
        let run = || {
            let journal = Journal::default();
            let mut engine = Engine::new(unpaced_config());
            engine.add_subsystem(Box::new(OrderProbe));
            engine.add_subsystem(Probe::boxed("a", &journal));
            engine.add_subsystem(Probe::boxed("b", &journal));
            engine.start();
            engine.on_update();
            engine.on_redraw();
            engine.on_update();
            engine.on_shutdown();
            let order = engine
                .context
                .world()
                .resource::<Order>()
                .map(|order| order.0.clone())
                .unwrap_or_default();
            (journal.entries(), order)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn events_are_visible_to_same_frame_simulation() {
        struct CountEvents;

        impl System for CountEvents {
            fn name(&self) -> &str {
                "count_events"
            }

            fn run(&self, world: &mut World) -> ChaosResult<()> {
                let seen = world.messages::<Event>().count() as u32;
                if let Some(ticks) = world.resource_mut::<Ticks>() {
                    ticks.0 += seen;
                }
                Ok(())
            }
        }

        struct EventCounterInstaller;

        impl Subsystem for EventCounterInstaller {
            fn name(&self) -> &str {
                "event_counter_installer"
            }

            fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
                context.world_mut().insert_resource(Ticks(0));
                context
                    .schedule_mut()
                    .add_system(stages::UPDATE, CountEvents)
            }
        }

        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Box::new(EventCounterInstaller));
        engine.start();
        engine.on_event(Event::Input(InputEvent::CursorEntered));
        engine.on_event(Event::Input(InputEvent::CursorLeft));
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Ticks>(), Some(&Ticks(2)));
    }

    struct FixedMark;

    impl System for FixedMark {
        fn name(&self) -> &str {
            "fixed_mark"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            if let Some(order) = world.resource_mut::<Order>() {
                order.0.push("fixed");
            }
            Ok(())
        }
    }

    struct FixedInstaller;

    impl Subsystem for FixedInstaller {
        fn name(&self) -> &str {
            "fixed_installer"
        }

        fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
            context.world_mut().insert_resource(Order::default());
            context
                .fixed_schedule_mut()
                .add_system(stages::FIXED_UPDATE, FixedMark)
        }
    }

    fn fixed_config(step: Duration) -> EngineConfig {
        EngineConfig {
            time: TimeConfig {
                target_fps: None,
                fixed_timestep: step,
            },
            ..EngineConfig::default()
        }
    }

    /// Garantit un delta RÉEL non nul avant le prochain tick : l'horloge
    /// monotone a une granularité (~42 ns sur Apple Silicon) et deux
    /// ticks dos à dos peuvent mesurer un delta ZÉRO — une frame sans
    /// pas fixe, et des comptes non déterministes sous charge (CI).
    fn let_real_time_pass() {
        std::thread::sleep(Duration::from_micros(50));
    }

    #[test]
    fn fixed_systems_run_at_a_fixed_bounded_cadence() {
        use chaos_core::FixedTime;

        // Pas MINUSCULE : l'accumulateur sature le cap à chaque frame —
        // comptes exacts, déterministes en headless.
        let mut engine = Engine::new(fixed_config(Duration::from_nanos(1)));
        engine.add_subsystem(Box::new(FixedInstaller));
        engine.start();
        let_real_time_pass();
        engine.on_update();
        let_real_time_pass();
        engine.on_update();
        let order = engine.context.world().resource::<Order>().unwrap();
        let cap = chaos_core::FixedClock::DEFAULT_MAX_STEPS as usize;
        assert_eq!(order.0.len(), 2 * cap);
        let fixed = engine.context.world().resource::<FixedTime>().unwrap();
        assert_eq!(fixed.delta, Duration::from_nanos(1));
        assert_eq!(fixed.step_index, (2 * cap) as u64);

        // Pas ÉNORME : zéro pas, le variable tourne quand même.
        let mut engine = Engine::new(fixed_config(Duration::from_secs(3600)));
        engine.add_subsystem(Box::new(FixedInstaller));
        engine.start();
        engine.on_update();
        let order = engine.context.world().resource::<Order>().unwrap();
        assert!(order.0.is_empty());
        assert_eq!(engine.context.time().frame_index, 1);
    }

    #[test]
    fn fixed_update_runs_before_the_variable_stages() {
        let mut engine = Engine::new(fixed_config(Duration::from_nanos(1)));
        engine.add_subsystem(Box::new(FixedInstaller));
        engine.add_subsystem(Box::new(OrderProbe));
        engine.start();
        let_real_time_pass();
        engine.on_update();
        let order = engine.context.world().resource::<Order>().unwrap();
        let cap = chaos_core::FixedClock::DEFAULT_MAX_STEPS as usize;
        let mut expected = vec!["fixed"; cap];
        expected.extend(["update", "late_update", "post_update", "subsystem"]);
        assert_eq!(order.0, expected);
    }

    #[test]
    fn time_scale_zero_is_not_a_pause() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.context.set_time_scale(0.0);
        engine.on_update();
        engine.on_update();
        let time = engine.context.time();
        assert_eq!(time.delta, Duration::ZERO);
        assert_eq!(time.elapsed, Duration::ZERO);
        assert_eq!(time.frame_index, 2);
        assert_eq!(time.scale, 0.0);
        // Les systèmes et subsystems TOURNENT — à delta nul (≠ pause).
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2"]
        );
    }

    #[test]
    fn pause_produces_no_phantom_fixed_steps() {
        let mut engine = Engine::new(fixed_config(Duration::from_nanos(1)));
        engine.add_subsystem(Box::new(FixedInstaller));
        engine.start();
        let_real_time_pass();
        engine.on_update();
        let before = engine.context.world().resource::<Order>().unwrap().0.len();
        engine.context.request_pause();
        engine.on_update();
        engine.on_update();
        assert_eq!(
            engine.context.world().resource::<Order>().unwrap().0.len(),
            before
        );
        engine.context.request_resume();
        engine.on_update();
        let_real_time_pass();
        engine.on_update();
        assert!(engine.context.world().resource::<Order>().unwrap().0.len() > before);
    }

    #[test]
    fn an_invalid_time_scale_is_refused() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.context.set_time_scale(f32::NAN);
        assert_eq!(engine.context.time_scale(), 1.0);
        engine.context.set_time_scale(-3.0);
        assert_eq!(engine.context.time_scale(), 0.0);
        engine.context.set_time_scale(2.0);
        engine.on_update();
        assert_eq!(engine.context.time().scale, 2.0);
    }

    #[test]
    fn pause_freezes_updates_but_keeps_rendering() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.context.request_pause();
        engine.on_update();
        engine.on_update();
        engine.on_redraw();
        assert!(engine.context.paused());
        assert_eq!(journal.entries(), vec!["init a", "update a 1", "render a"]);
    }

    #[test]
    fn resume_restarts_at_the_frame_boundary_without_time_jump() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.context.request_pause();
        engine.on_update();
        engine.on_update();
        engine.context.request_resume();
        engine.on_update();
        assert!(!engine.context.paused());
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "update a 2"]
        );
    }

    #[test]
    fn time_is_frozen_while_paused() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        let frozen = engine.context.world().resource::<Time>().copied();
        engine.context.request_pause();
        engine.on_update();
        engine.on_update();
        assert_eq!(engine.context.world().resource::<Time>().copied(), frozen);
        assert_eq!(engine.context.time().frame_index, 1);
    }

    #[test]
    fn messages_do_not_accumulate_while_paused() {
        let mut engine = Engine::new(unpaced_config());
        engine.start();
        engine.on_update();
        engine.context.request_pause();
        engine.on_update();
        for _ in 0..3 {
            engine.on_event(Event::Input(InputEvent::CursorEntered));
        }
        engine.on_update();
        assert_eq!(engine.context.world().messages::<Event>().count(), 0);
    }

    #[test]
    fn a_pause_request_before_start_is_purged() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.context.request_pause();
        engine.start();
        engine.on_update();
        assert!(!engine.context.paused());
        assert_eq!(journal.entries(), vec!["init a", "update a 1"]);
    }

    #[test]
    fn an_irrelevant_pause_request_is_discarded() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.context.request_resume();
        engine.on_update();
        assert!(!engine.context.paused());
        assert_eq!(journal.entries(), vec!["init a", "update a 1"]);
    }

    #[test]
    fn starting_twice_is_explicitly_refused() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.start();
        engine.on_update();
        assert_eq!(journal.entries(), vec!["init a", "update a 1"]);
    }

    #[test]
    fn repeated_shutdown_is_idempotent_and_the_engine_stays_silent_after() {
        let journal = Journal::default();
        let mut engine = Engine::new(unpaced_config());
        engine.add_subsystem(Probe::boxed("a", &journal));
        engine.start();
        engine.on_update();
        engine.on_shutdown();
        engine.on_shutdown();
        engine.on_update();
        engine.on_redraw();
        engine.on_event(Event::Input(InputEvent::CursorEntered));
        assert_eq!(
            journal.entries(),
            vec!["init a", "update a 1", "shutdown a"]
        );
    }

    #[test]
    fn the_engine_loads_a_real_scene_file_end_to_end() {
        use chaos_assets::{AssetKind, AssetSource};
        use chaos_core::math::Vec3;
        use chaos_core::{GlobalTransform, Transform};
        use chaos_scene::{EntityData, FORMAT_VERSION, SceneData};

        let path = std::env::temp_dir().join(format!(
            "chaos_engine_scene_e2e_{}.cscn",
            std::process::id()
        ));
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("scenes/e2e"),
            entities: vec![
                EntityData {
                    transform: Some(Transform::from_translation(Vec3::new(2.0, 0.0, 0.0))),
                    mesh: None,
                    parent: None,
                },
                EntityData {
                    transform: Some(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0))),
                    mesh: None,
                    parent: Some(0),
                },
            ],
        };
        crate::scenes::save_scene(&path, &data).unwrap();

        let mut engine = Engine::new(unpaced_config());
        engine.start();
        let asset = engine
            .context
            .assets_mut()
            .declare(
                "scenes/e2e",
                AssetKind::Scene,
                AssetSource::File(path.clone()),
            )
            .unwrap();
        let loaded = crate::scenes::load_scene(engine.context.assets_mut(), asset).unwrap();
        let (world, scenes) = engine.context.world_and_scenes();
        let id = scenes.create(&loaded.name).unwrap();
        scenes
            .load(world, id, |scene, world| loaded.apply(scene, world))
            .unwrap();
        scenes.activate(id).unwrap();
        engine.on_update();

        let world = engine.context.world();
        let scene = engine.context.scenes().scene(id).unwrap();
        assert_eq!(scene.members(world).count(), 2);
        let child = scene
            .members(world)
            .find(|entity| {
                world
                    .get::<GlobalTransform>(*entity)
                    .is_some_and(|global| (global.translation().x - 3.0).abs() < 1e-5)
            })
            .expect("l'enfant composé doit porter un global frais");
        assert!(world.is_alive(child));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_failed_scene_load_leaves_the_engine_coherent() {
        use chaos_assets::{AssetKind, AssetSource};

        let path = std::env::temp_dir().join(format!(
            "chaos_engine_scene_corrupt_{}.cscn",
            std::process::id()
        ));
        fs::write(&path, b"definitely not a scene").unwrap();

        let mut engine = Engine::new(unpaced_config());
        engine.start();
        let asset = engine
            .context
            .assets_mut()
            .declare(
                "scenes/corrupt",
                AssetKind::Scene,
                AssetSource::File(path.clone()),
            )
            .unwrap();
        let error = crate::scenes::load_scene(engine.context.assets_mut(), asset).unwrap_err();
        assert!(error.to_string().contains("malformed scene file"));
        assert!(engine.context.world().is_empty());
        assert!(engine.context.scenes().is_empty());
        engine.on_update();
        assert!(!engine.exit_requested());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_engine_propagates_transforms_each_update() {
        use chaos_core::math::Vec3;
        use chaos_core::{GlobalTransform, Transform};
        use chaos_scene::hierarchy;

        let mut engine = Engine::new(unpaced_config());
        engine.start();
        let world = engine.context.world_mut();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        hierarchy::attach(world, child, parent).unwrap();
        engine.on_update();
        let global = engine
            .context
            .world()
            .get::<GlobalTransform>(child)
            .unwrap();
        assert_eq!(global.translation(), Vec3::new(3.0, 0.0, 0.0));
    }
}
