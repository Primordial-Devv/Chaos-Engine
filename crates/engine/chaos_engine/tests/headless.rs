use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chaos_engine::{
    ChaosResult, Engine, EngineConfig, EngineContext, EntityData, FORMAT_VERSION, FixedTime,
    GlobalTransform, Resource, RuntimeConfig, SceneData, Subsystem, System, TimeConfig, Transform,
    World,
    assets::{AssetKind, AssetSource},
    math::Vec3,
    scenes, stages,
};

#[derive(Debug, Default, PartialEq)]
struct FixedSteps(u64);

impl Resource for FixedSteps {}

struct Drift;

impl System for Drift {
    fn name(&self) -> &str {
        "app.drift"
    }

    fn run(&self, world: &mut World) -> ChaosResult<()> {
        for (_, transform) in world.query_mut::<Transform>() {
            transform.translation.x += 1.0;
        }
        Ok(())
    }
}

struct CountSteps;

impl System for CountSteps {
    fn name(&self) -> &str {
        "app.count_steps"
    }

    fn run(&self, world: &mut World) -> ChaosResult<()> {
        let index = world
            .resource::<FixedTime>()
            .map(|fixed| fixed.step_index)
            .unwrap_or_default();
        if let Some(steps) = world.resource_mut::<FixedSteps>() {
            steps.0 = index;
        }
        Ok(())
    }
}

struct HeadlessApp {
    journal: Rc<RefCell<Vec<String>>>,
    steps_per_tick: Rc<RefCell<Vec<u64>>>,
    scene_path: PathBuf,
}

impl Subsystem for HeadlessApp {
    fn name(&self) -> &str {
        "headless_app"
    }

    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        assert!(context.renderer().is_none());
        let asset = context.assets_mut().declare(
            "scenes/headless",
            AssetKind::Scene,
            AssetSource::File(self.scene_path.clone()),
        )?;
        let data = scenes::load_scene(context.assets_mut(), asset)?;
        let (world, managers) = context.world_and_scenes();
        let id = managers.create(&data.name)?;
        managers.load(world, id, |scene, world| data.apply(scene, world))?;
        managers.activate(id)?;
        context.world_mut().insert_resource(FixedSteps::default());
        context.schedule_mut().add_system(stages::UPDATE, Drift)?;
        context
            .fixed_schedule_mut()
            .add_system(stages::FIXED_UPDATE, CountSteps)?;
        self.journal.borrow_mut().push(String::from("init"));
        Ok(())
    }

    fn update(&mut self, context: &mut EngineContext) {
        let frame = context.time().frame_index;
        let world = context.world();
        let max_x = world
            .query::<GlobalTransform>()
            .map(|(_, global)| global.translation().x)
            .fold(f32::MIN, f32::max);
        let steps = world
            .resource::<FixedSteps>()
            .map(|steps| steps.0)
            .unwrap_or_default();
        self.journal.borrow_mut().push(format!(
            "tick {frame} entities {} max_x {max_x}",
            world.len()
        ));
        self.steps_per_tick.borrow_mut().push(steps);
        // Un delta réel non nul garanti avant le tick suivant : l'horloge
        // monotone a une granularité et un tick à delta zéro perdrait ses
        // pas fixes — les comptes différentiels resteraient exacts mais
        // le sommeil rend chaque tick 2..N déterministe.
        std::thread::sleep(Duration::from_micros(50));
    }

    fn render(&mut self, _context: &mut EngineContext) {
        self.journal.borrow_mut().push(String::from("render"));
    }

    fn shutdown(&mut self, context: &mut EngineContext) {
        assert!(context.renderer().is_none());
        self.journal.borrow_mut().push(String::from("shutdown"));
    }
}

/// LE checkpoint de la sous-phase headless, par l'API publique seule :
/// une application complète (scène réelle chargée via l'Asset Pipeline,
/// système variable, système à pas fixe, hiérarchie propagée) démarre,
/// exécute exactement N ticks sans fenêtre ni GPU, et s'arrête proprement
/// — la phase présentation n'existe pas (aucun `render`).
#[test]
fn a_complete_headless_application_runs_its_ticks_and_stops_cleanly() {
    let scene_path =
        std::env::temp_dir().join(format!("chaos_headless_e2e_{}.cscn", std::process::id()));
    let data = SceneData {
        version: FORMAT_VERSION,
        name: String::from("scenes/headless"),
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
    scenes::save_scene(&scene_path, &data).unwrap();

    let journal: Rc<RefCell<Vec<String>>> = Rc::default();
    let steps_per_tick: Rc<RefCell<Vec<u64>>> = Rc::default();
    let mut engine = Engine::new(EngineConfig {
        time: TimeConfig {
            target_fps: None,
            fixed_timestep: Duration::from_nanos(1),
        },
        runtime: RuntimeConfig {
            headless: true,
            frame_limit: Some(10),
            ..RuntimeConfig::default()
        },
        ..EngineConfig::default()
    });
    engine.add_subsystem(Box::new(HeadlessApp {
        journal: journal.clone(),
        steps_per_tick: steps_per_tick.clone(),
        scene_path: scene_path.clone(),
    }));

    assert_eq!(engine.run(), Ok(()));

    let entries = journal.borrow().clone();
    assert_eq!(entries.len(), 12);
    assert_eq!(entries.first().map(String::as_str), Some("init"));
    assert_eq!(entries.last().map(String::as_str), Some("shutdown"));
    assert!(!entries.iter().any(|entry| entry == "render"));
    assert_eq!(entries[1], "tick 1 entities 2 max_x 5");
    assert_eq!(entries[10], "tick 10 entities 2 max_x 23");

    let steps = steps_per_tick.borrow().clone();
    assert_eq!(steps.len(), 10);
    assert!(steps[0] <= 5);
    assert_eq!(steps[9] - steps[0], 45);
    assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]));
    let _ = fs::remove_file(&scene_path);
}
