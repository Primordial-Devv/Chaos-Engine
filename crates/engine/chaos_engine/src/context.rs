use std::time::Instant;

use chaos_assets::AssetManager;
use chaos_core::{ChaosError, ChaosResult, FixedClock, Time};
use chaos_ecs::{Schedule, World};
use chaos_renderer::Renderer;
use chaos_scene::SceneManager;
use log::{error, warn};

use crate::diagnostics::FrameDiagnostics;
use crate::metrics::EngineMetrics;

/// La CARTE DES SERVICES du moteur — l'interface offerte aux subsystems
/// pendant leurs hooks, jamais un sac global (la carte complète et ses
/// règles : `docs/architecture/engine-loop.md`) :
///
/// - **temps** : `time()` (+ ressources `Time`/`FixedTime` du World) ;
///   l'échelle par requête à frontière de frame ;
/// - **World ECS** : les données vivantes ET le canal de communication
///   inter-subsystems (messages/ressources) — jamais de dépendance
///   directe entre subsystems ;
/// - **schedulers** : variable et fixe — enregistrement à l'init
///   (recommandé) ; post-init : appliqué à la frame suivante,
///   déterministe à configuration identique ;
/// - **assets** : l'unique point d'entrée I/O ;
/// - **scènes** : le manager, seul mutateur d'états ; l'emprunt scindé
///   fourni (`world_and_scenes`) ;
/// - **renderer** : `Option` — l'Option EST la frontière headless ;
/// - **arrêt/pause/échelle** : des requêtes appliquées à la frontière de
///   frame, jamais d'effet immédiat ;
/// - **défaillance fatale** : `report_fatal` — l'escalade d'un subsystem
///   qui ne peut pas continuer (arrêt ordonné + diagnostic remonté par
///   `Engine::run`) ; le récupérable se traite localement, sans arrêt ;
/// - **diagnostics** : `diagnostics()` — le profil CPU de la dernière
///   frame complète (phases, subsystems, budget), en lecture seule ;
/// - **metrics** : `metrics()` — la santé synthétique et continue
///   (FPS, jauges, compteurs, états des subsystems), en lecture seule.
///
/// Les mutateurs du moteur restent `pub(crate)` ; aucun état global caché
/// nulle part (verrouillé en CI par `tests/boundaries.rs`).
pub struct EngineContext {
    time: Time,
    time_scale: f32,
    exit_requested: bool,
    pause_request: Option<bool>,
    paused: bool,
    fatal: Option<ChaosError>,
    diagnostics: FrameDiagnostics,
    metrics: EngineMetrics,
    renderer: Option<Renderer>,
    assets: AssetManager,
    world: World,
    schedule: Schedule,
    fixed: Schedule,
    scenes: SceneManager,
}

impl Default for EngineContext {
    fn default() -> Self {
        Self {
            time: Time::default(),
            time_scale: 1.0,
            exit_requested: false,
            pause_request: None,
            paused: false,
            fatal: None,
            diagnostics: FrameDiagnostics::default(),
            metrics: EngineMetrics::default(),
            renderer: None,
            assets: AssetManager::default(),
            world: World::default(),
            schedule: Schedule::default(),
            fixed: Schedule::default(),
            scenes: SceneManager::default(),
        }
    }
}

impl EngineContext {
    pub fn time(&self) -> Time {
        self.time
    }

    /// Demande l'arrêt propre du moteur à la fin de la frame courante.
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// LE canal d'escalade d'un subsystem qui ne peut pas continuer — le
    /// pendant FATAL de `request_exit` : le diagnostic est conservé et
    /// remonté par `Engine::run()`, l'arrêt ordonné suit à la frontière
    /// de frame (nettoyage complet). La PREMIÈRE défaillance est LA
    /// cause ; les suivantes sont loguées comme conséquences, jamais
    /// perdues en silence, jamais écrasantes. Pour une erreur
    /// RÉCUPÉRABLE, ne pas escalader : la traiter localement — le moteur
    /// continue.
    pub fn report_fatal(&mut self, error: ChaosError) {
        if self.fatal.is_none() {
            error!(
                "fatal failure at frame {}: {error} — ordered shutdown requested",
                self.time.frame_index
            );
        }
        self.store_fatal(error);
    }

    /// Le stockage de la défaillance SANS log de primaire — les chemins
    /// moteur gardent leurs `error!` spécifiques ; une conséquence
    /// écartée est quand même loguée (jamais une perte silencieuse).
    pub(crate) fn store_fatal(&mut self, error: ChaosError) {
        self.exit_requested = true;
        self.metrics.count_error();
        match &self.fatal {
            None => self.fatal = Some(error),
            Some(primary) => {
                error!(
                    "subsequent failure (the first failure stays the cause: {primary}): {error}"
                );
            }
        }
    }

    pub(crate) fn take_fatal(&mut self) -> Option<ChaosError> {
        self.fatal.take()
    }

    #[cfg(test)]
    pub(crate) fn fatal(&self) -> Option<&ChaosError> {
        self.fatal.as_ref()
    }

    /// Demande la mise en pause de la simulation — appliquée par le moteur
    /// à la frontière de frame (point déterministe). En pause : horloge,
    /// schedule et updates gelés ; le rendu continue, les événements
    /// circulent (c'est par eux que la reprise arrive).
    pub fn request_pause(&mut self) {
        self.pause_request = Some(true);
    }

    /// Demande la reprise de la simulation — appliquée à la frontière de
    /// frame. L'horloge est RESYNCHRONISÉE à la reprise : delta quasi
    /// nul, aucun saut de simulation, aucune rafale de pas fixes ; la
    /// durée de la pause reste créditée au temps réel (`real_elapsed`).
    pub fn request_resume(&mut self) {
        self.pause_request = Some(false);
    }

    /// La simulation est-elle en pause ? (Miroir posé par le moteur.)
    pub fn paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn take_pause_request(&mut self) -> Option<bool> {
        self.pause_request.take()
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Les diagnostics du moteur : le profil CPU de la dernière frame
    /// complète (`last_frame()`) et les dépassements de budget cumulés —
    /// la lecture du futur profiler de l'éditeur et des outils.
    pub fn diagnostics(&self) -> &FrameDiagnostics {
        &self.diagnostics
    }

    pub(crate) fn diagnostics_mut(&mut self) -> &mut FrameDiagnostics {
        &mut self.diagnostics
    }

    /// Les metrics de santé du moteur : la photo synthétique et continue
    /// (`snapshot()`) — FPS, temps de frame, jauges, compteurs, états des
    /// subsystems — découplée de toute UI.
    pub fn metrics(&self) -> &EngineMetrics {
        &self.metrics
    }

    pub(crate) fn metrics_mut(&mut self) -> &mut EngineMetrics {
        &mut self.metrics
    }

    pub fn renderer(&self) -> Option<&Renderer> {
        self.renderer.as_ref()
    }

    pub fn renderer_mut(&mut self) -> Option<&mut Renderer> {
        self.renderer.as_mut()
    }

    /// L'Asset Manager du moteur — l'unique point d'entrée des ressources.
    pub fn assets(&self) -> &AssetManager {
        &self.assets
    }

    pub fn assets_mut(&mut self) -> &mut AssetManager {
        &mut self.assets
    }

    /// Le monde ECS du moteur : entités, composants, ressources et
    /// messages, partagés par les systèmes et les subsystems.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// L'ordonnancement des systèmes ECS à temps VARIABLE — les subsystems
    /// y enregistrent leurs systèmes pendant `init`
    /// (`stages::{UPDATE, LATE_UPDATE, POST_UPDATE}`).
    pub fn schedule_mut(&mut self) -> &mut Schedule {
        &mut self.schedule
    }

    /// L'ordonnancement des systèmes à PAS FIXE (`stages::FIXED_UPDATE`) —
    /// exécuté 0..N fois par frame (rattrapage borné) ; ses systèmes
    /// lisent la ressource `FixedTime`, jamais `Time`.
    pub fn fixed_schedule_mut(&mut self) -> &mut Schedule {
        &mut self.fixed
    }

    /// Fixe l'échelle de temps — appliquée par le moteur au tick suivant
    /// (frontière de frame). Une valeur non finie est refusée avec un
    /// avertissement ; une valeur négative devient 0.
    pub fn set_time_scale(&mut self, scale: f32) {
        if !scale.is_finite() {
            warn!(
                "time scale {scale} is not finite, keeping {}",
                self.time_scale
            );
            self.metrics.count_warning();
            return;
        }
        self.time_scale = scale.max(0.0);
    }

    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// Le point d'entrée unique de la gestion des scènes.
    pub fn scenes(&self) -> &SceneManager {
        &self.scenes
    }

    pub fn scenes_mut(&mut self) -> &mut SceneManager {
        &mut self.scenes
    }

    /// Le monde ET le manager de scènes, empruntés ensemble — le besoin de
    /// `load`/`unload`/`replace` (emprunts disjoints par champs).
    pub fn world_and_scenes(&mut self) -> (&mut World, &mut SceneManager) {
        (&mut self.world, &mut self.scenes)
    }

    /// Le nettoyage du shutdown : toutes les scènes déchargées, registre
    /// vidé — garanti par le moteur.
    pub(crate) fn shutdown_scenes(&mut self) -> ChaosResult<()> {
        self.scenes.shutdown(&mut self.world)
    }

    /// La fermeture des assets à l'arrêt : caches vidés, rétentions
    /// oubliées, états `Loaded` → `Unloaded` — garanti par le moteur.
    pub(crate) fn shutdown_assets(&mut self) {
        self.assets.shutdown();
    }

    /// La remise à zéro du World à l'arrêt — la garantie PAR
    /// CONSTRUCTION : plus une entité (globales et persistantes
    /// comprises), plus une ressource, plus un message en attente. Le
    /// slot de défaillance fatale n'est PAS touché (il survit à l'arrêt,
    /// `run()` le draine).
    pub(crate) fn reset_world(&mut self) {
        self.world = World::default();
    }

    /// Le tick ECS de la frame : la ressource `Time` rafraîchie, puis les
    /// PAS FIXES (0..N, `FixedTime` rafraîchie à chaque pas), puis le
    /// schedule variable stage par stage — avant les updates des
    /// subsystems, qui lisent l'état frais. Chaque bloc est chronométré
    /// dans le profil de la frame courante.
    pub(crate) fn tick_world(
        &mut self,
        time: Time,
        fixed_clock: &mut FixedClock,
        steps: u32,
    ) -> ChaosResult<()> {
        self.world.insert_resource(time);
        let fixed_start = Instant::now();
        for _ in 0..steps {
            self.world.insert_resource(fixed_clock.step());
            self.fixed.run(&mut self.world)?;
        }
        self.diagnostics.record_fixed(steps, fixed_start.elapsed());
        for index in 0..self.schedule.stage_count() {
            let stage_start = Instant::now();
            self.schedule.run_stage_at(index, &mut self.world)?;
            if let Some(name) = self.schedule.stage_name_at(index) {
                self.diagnostics
                    .record_stage(index, name, stage_start.elapsed());
            }
        }
        Ok(())
    }

    pub(crate) fn set_time(&mut self, time: Time) {
        self.time = time;
    }

    pub(crate) fn set_renderer(&mut self, renderer: Renderer) {
        self.renderer = Some(renderer);
    }

    pub(crate) fn take_renderer(&mut self) -> Option<Renderer> {
        self.renderer.take()
    }
}
