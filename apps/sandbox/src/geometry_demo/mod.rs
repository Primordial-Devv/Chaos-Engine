//! La DÉMO du sandbox, organisée en RIGS — un module par preuve vivante
//! du renderer, le subsystem n'est plus qu'un orchestrateur mince :
//!
//! | Module | Preuve vivante |
//! |---|---|
//! | [`stage`] | la scène-fichier (Asset Pipeline + ECS), la ronde, les triangles |
//! | [`mirror`] | l'écran de surveillance (cible hors écran + passe déclarée) |
//! | [`lighting`] | l'ambiante, les OMBRES, le soleil (K/N), les ponctuelles + marqueurs, le spot |
//! | [`pbr`] | la grille metallic × roughness, le cube normal-mappé, la sphère émissive pulsante |
//! | [`environment`] | la cubemap HDR, le ciel, l'IBL, l'exposition (E, V/B) |
//! | [`opacity`] | le trio de verres TRIÉS par profondeur, la grille MASKED |
//! | [`swarm`] | l'essaim de 1 200 mini-cubes — l'instancing automatique |
//! | [`debug`] | le debug rendering : grille, axes, bounds, frustums, lumières (G/X/F/J/T) |
//! | [`content`] | les pixels procéduraux partagés |
//! | [`spin`] | le comportement ECS du cube central |
//!
//! L'ORDRE COMPTE et il est préservé : la construction des rigs suit la
//! séquence historique de création des ressources (les logs debug et
//! `docs/testing.md` s'y réfèrent), et l'update soumet lumières puis
//! draws dans l'ordre de scène historique.
//!
//! Le PLAN DE LA SCÈNE — des PAVILLONS d'exposition espacés sur le sol
//! 20×20, la caméra apparaît en (0, 2.2, 10.5) face au centre :
//!
//! ```text
//!        z = -10 ───────────── LE FOND ─────────────
//!   triangles       bumpy   [grille PBR 4×4]   sphère
//!   (totem,         cube      metallic ×       émissive
//!    coin -7,-7)    (-3)      roughness (z=-7) (+3)
//!
//!   x = -10                                      x = +10
//!   [écran du      LE CENTRE : cube + satellites, [verres ×3
//!    miroir        ronde r=2.2, lumières          étagés en z,
//!    en x=-7]      orbitales ≤ 4.5, spot          x=+7, au sol]
//!                                                 [grille masked
//!                                                  devant, z=+3.2]
//!        z = +10 ───────── LE DEVANT (caméra) ─────────
//! ```

mod content;
mod debug;
mod environment;
mod lighting;
mod mirror;
mod opacity;
mod pbr;
mod spin;
mod stage;
mod swarm;

use chaos_engine::{
    Camera, ChaosResult, ElementState, EngineContext, Event, InputEvent, KeyCode, Subsystem,
    WindowEvent, debug::DebugCameraController, math::Vec3,
};

use debug::DebugRig;
use environment::Environment;
use lighting::Lighting;
use mirror::Mirror;
use opacity::Opacity;
use pbr::PbrShowcase;
use stage::Stage;
use swarm::Swarm;

/// Démo multi-objets pilotée par les MATERIALS : chaque draw est le triplet
/// mesh + material + transform. **Le sol vient de fichiers via l'Asset
/// Pipeline** (texture `assets/textures/checker.ppm`, mesh
/// `assets/models/floor.glb` — déclarés, importés puis cousus vers le
/// renderer par `chaos_engine::assets`) ; le reste est procédural. Les
/// materials sont DESCRIPTIFS (modèle + paramètres + état — AUCUN
/// pipeline explicite : les permutations se résolvent seules). **La scène
/// est ÉCLAIRÉE** (ambiante douce, directionnelle chaude togglée par K,
/// trois ponctuelles orbitantes suivies de leurs marqueurs, spot cyan),
/// **OMBRÉE** (shadow map 2048 sur volume explicite, togglée par N),
/// **ENVIRONNÉE** (cubemap HDR procédurale : ciel + IBL, togglée par E,
/// exposition V/B) et **ORCHESTRÉE en deux passes déclarées**
/// (`demo.mirror` ordre -10, puis `chaos.main`). S'y montrent le
/// showcase PBR, le trio de verres transparents triés par profondeur et
/// la grille masked — le détail de chaque rig vit dans son module.
/// **Le sol, le cube central et ses deux satellites sont du CONTENU : la
/// scène `scenes/demo` est chargée depuis le fichier committé
/// `assets/scenes/demo.cscn` — aucune entité n'est construite dans ce
/// code** ; le cube tourne via son composant `Spin` animé par le système
/// `demo.spin`, les satellites suivent par la seule propagation des
/// transforms.
/// Lancer depuis la racine du workspace (chemins d'assets relatifs).
/// N'utilise que le vocabulaire haut niveau — l'API d'un futur gamemode.
#[derive(Default)]
pub struct GeometryDemo {
    rigs: Option<Rigs>,
    camera: Camera,
    controller: DebugCameraController,
    elapsed: f32,
    /// `CHAOS_DIAG_FRAME=<n>` : le snapshot des diagnostics du renderer
    /// est loggé à la frame n — la procédure de validation GPU
    /// répétable (longs runs, comparaisons entre machines), sans
    /// instrumentation temporaire.
    diag_frame: Option<u32>,
    frames: u32,
}

/// Les rigs de la démo, construits ensemble à l'init (fenêtré
/// seulement) — chacun possède SES handles : plus un champ optionnel à
/// plat, un groupe par responsabilité.
struct Rigs {
    stage: Stage,
    mirror: Mirror,
    lighting: Lighting,
    pbr: PbrShowcase,
    environment: Environment,
    opacity: Opacity,
    swarm: Swarm,
    debug: DebugRig,
}

impl Subsystem for GeometryDemo {
    fn name(&self) -> &str {
        "geometry_demo"
    }

    fn requires_graphics(&self) -> bool {
        true
    }

    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        if context.renderer().is_none() {
            return Ok(());
        }
        self.diag_frame = std::env::var("CHAOS_DIAG_FRAME")
            .ok()
            .and_then(|value| value.parse().ok());

        // Les rigs se construisent dans l'ORDRE HISTORIQUE de création
        // des ressources : la scène de fond, le miroir (cible + écran),
        // l'opacité (verres + grille), l'environnement (ciel), puis
        // l'éclairage (ambiante + ombres + marqueurs), le showcase PBR,
        // et la passe miroir déclarée en dernier.
        let Some(mut stage) = Stage::build(context)? else {
            return Ok(());
        };
        let Some(renderer) = context.renderer_mut() else {
            return Ok(());
        };
        let mut mirror = Mirror::build(renderer)?;
        let opacity = Opacity::build(renderer)?;
        let environment = Environment::build(renderer)?;
        let lighting = Lighting::build(renderer)?;
        let pbr = PbrShowcase::build(renderer)?;
        let swarm = Swarm::build(renderer)?;
        let debug = DebugRig::build(renderer);
        mirror.declare_pass(renderer)?;

        self.camera.transform.translation = Vec3::new(0.0, 2.2, 10.5);
        let (width, height) = renderer.surface_size();
        self.camera.set_viewport(width, height);

        stage.populate_scene(context)?;

        self.rigs = Some(Rigs {
            stage,
            mirror,
            lighting,
            pbr,
            environment,
            opacity,
            swarm,
            debug,
        });
        Ok(())
    }

    fn on_event(&mut self, event: &Event, context: &mut EngineContext) {
        self.controller.handle_event(event);
        if let Event::Window(WindowEvent::Resized { width, height }) = event {
            self.camera.set_viewport(*width, *height);
        }
        // P bascule la pause moteur : simulation gelée, fenêtre vivante.
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::P,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
        {
            if context.paused() {
                context.request_resume();
            } else {
                context.request_pause();
            }
        }
        // L bascule le slow-motion (échelle de temps 1.0 ↔ 0.25) — tout le
        // temps de JEU ralentit, caméra comprise ; le temps réel, jamais.
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::L,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
        {
            let scale = if context.time_scale() < 1.0 {
                1.0
            } else {
                0.25
            };
            context.set_time_scale(scale);
        }
        // O affiche le rapport de la dernière frame complète — le chemin
        // de lecture du futur profiler, à la demande (jamais en continu)
        // — suivi du SNAPSHOT des diagnostics du renderer : ce que la
        // frame a rendu, éliminé, possédé et coûté (CPU mesuré, GPU réel
        // quand disponible), en lignes de log lisibles.
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::O,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
        {
            log::info!("{}", context.diagnostics().last_frame());
            if let Some(renderer) = context.renderer_mut() {
                log::info!("{}", renderer.diagnostics());
            }
        }
        // H affiche la santé synthétique — le chemin de lecture du futur
        // overlay debug et des diagnostics de production.
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::H,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
        {
            log::info!("{}", context.metrics().snapshot());
        }
        // Les toggles des rigs : K (soleil), E (environnement), N
        // (ombres), V/B (exposition) — chacun délégué à son rig.
        let Some(rigs) = &mut self.rigs else {
            return;
        };
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::K,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
        {
            rigs.lighting.toggle_sun();
        }
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::E,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
            && let Some(renderer) = context.renderer_mut()
        {
            rigs.environment.toggle(renderer);
        }
        if let Event::Input(InputEvent::Keyboard {
            key: KeyCode::N,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
            && let Some(renderer) = context.renderer_mut()
        {
            rigs.lighting.toggle_shadows(renderer);
        }
        if let Event::Input(InputEvent::Keyboard {
            key,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
            && let Some(factor) = match key {
                KeyCode::V => Some(1.0 / 1.25),
                KeyCode::B => Some(1.25),
                _ => None,
            }
            && let Some(renderer) = context.renderer_mut()
        {
            rigs.environment.adjust_exposure(renderer, factor);
        }
        // Le DEBUG RENDERING : G (tout), X (bounds), F (frustums),
        // J (lumières), T (marqueur 3 s à la caméra).
        if let Event::Input(InputEvent::Keyboard {
            key,
            state: ElementState::Pressed,
            repeat: false,
        }) = event
            && let Some(renderer) = context.renderer_mut()
        {
            match key {
                KeyCode::G => rigs.debug.toggle_global(renderer),
                KeyCode::X => rigs.debug.toggle_bounds(renderer),
                KeyCode::F => rigs.debug.toggle_frustums(renderer),
                KeyCode::J => rigs.debug.toggle_lights(renderer),
                KeyCode::T => rigs
                    .debug
                    .drop_marker(renderer, self.camera.transform.translation),
                _ => {}
            }
        }
    }

    fn update(&mut self, context: &mut EngineContext) {
        let Some(rigs) = &mut self.rigs else {
            return;
        };
        let delta_seconds = context.time().delta_seconds();
        self.controller.update(&mut self.camera, delta_seconds);
        self.elapsed += delta_seconds;
        let t = self.elapsed;

        // Le monde se lit AVANT d'emprunter le renderer.
        let scene_draws = rigs.stage.collect_scene_draws(context);
        let Some(renderer) = context.renderer_mut() else {
            return;
        };
        // Le levier de validation : le snapshot loggé à LA frame
        // demandée (le rapport de la frame précédente, complète).
        self.frames += 1;
        if self.diag_frame == Some(self.frames) {
            log::info!(
                "diagnostics at frame {}:\n{}",
                self.frames,
                renderer.diagnostics()
            );
        }
        renderer.set_view_projection(self.camera.view_projection());
        renderer.set_camera_position(self.camera.transform.translation);

        // L'ordre de soumission historique : les lumières (et leurs
        // marqueurs), la scène de fond (membres, ronde, triangles), le
        // miroir (la ronde redessinée + l'écran), le showcase PBR, puis
        // l'opacité (verres + grille), l'essaim et le debug.
        rigs.lighting.frame(renderer, t);
        let ring_transforms = rigs.stage.frame(renderer, &scene_draws, t);
        rigs.mirror.frame(renderer, &rigs.stage, &ring_transforms);
        rigs.pbr.frame(renderer, t);
        rigs.opacity.frame(renderer, t);
        rigs.swarm.frame(renderer, t);
        rigs.debug.frame(
            renderer,
            &rigs.stage,
            &rigs.mirror,
            &rigs.lighting,
            &ring_transforms,
            t,
        );
        // Le temps du debug avance avec la simulation : les marqueurs
        // retenus (T) expirent seuls.
        renderer.advance_debug_time(delta_seconds);
    }
}
