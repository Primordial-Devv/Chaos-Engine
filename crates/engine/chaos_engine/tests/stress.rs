use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chaos_engine::{
    ChaosResult, Engine, EngineConfig, EngineContext, Message, RuntimeConfig, Subsystem, System,
    TimeConfig, Transform, World, math::Vec3, stages,
};

fn headless(frame_limit: u64) -> EngineConfig {
    EngineConfig {
        time: TimeConfig {
            target_fps: None,
            ..TimeConfig::default()
        },
        runtime: RuntimeConfig {
            headless: true,
            frame_limit: Some(frame_limit),
            ..RuntimeConfig::default()
        },
        ..EngineConfig::default()
    }
}

/// 10 000 ticks headless non cadencés : le moteur tient la longueur, les
/// compteurs et le snapshot restent exacts.
#[test]
fn ten_thousand_headless_ticks_stay_stable() {
    struct Counter {
        ticks: Arc<AtomicU64>,
    }
    impl Subsystem for Counter {
        fn name(&self) -> &str {
            "counter"
        }
        fn update(&mut self, _context: &mut EngineContext) {
            self.ticks.fetch_add(1, Ordering::Relaxed);
        }
    }

    let ticks = Arc::new(AtomicU64::new(0));
    let mut engine = Engine::new(headless(10_000));
    engine.add_subsystem(Box::new(Counter {
        ticks: ticks.clone(),
    }));
    assert_eq!(engine.run(), Ok(()));
    assert_eq!(ticks.load(Ordering::Relaxed), 10_000);
    let snapshot = engine.metrics().snapshot();
    assert_eq!(snapshot.frame_index, 10_000);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(engine.diagnostics().last_frame().frame_index, 9_999);
    assert_eq!(engine.diagnostics().overruns(), 0);
}

/// 100 subsystems en chaîne de dépendances, enregistrés à l'ENVERS : le
/// tri reste exact, l'init suit la chaîne, le shutdown la remonte.
#[test]
fn a_hundred_subsystems_form_a_reliable_chain() {
    struct Link {
        name: &'static str,
        deps: Vec<&'static str>,
        journal: Arc<Mutex<Vec<String>>>,
    }
    impl Subsystem for Link {
        fn name(&self) -> &str {
            self.name
        }
        fn dependencies(&self) -> &[&str] {
            &self.deps
        }
        fn init(&mut self, _context: &mut EngineContext) -> ChaosResult<()> {
            self.journal
                .lock()
                .unwrap()
                .push(format!("i {}", self.name));
            Ok(())
        }
        fn update(&mut self, _context: &mut EngineContext) {
            self.journal
                .lock()
                .unwrap()
                .push(format!("u {}", self.name));
        }
        fn shutdown(&mut self, _context: &mut EngineContext) {
            self.journal
                .lock()
                .unwrap()
                .push(format!("s {}", self.name));
        }
    }

    let names: Vec<&'static str> = (0..100)
        .map(|index| -> &'static str { Box::leak(format!("link{index:03}").into_boxed_str()) })
        .collect();
    let journal: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut engine = Engine::new(headless(3));
    for index in (0..100).rev() {
        let deps = if index == 0 {
            Vec::new()
        } else {
            vec![names[index - 1]]
        };
        engine.add_subsystem(Box::new(Link {
            name: names[index],
            deps,
            journal: journal.clone(),
        }));
    }
    assert_eq!(engine.run(), Ok(()));
    let entries = journal.lock().unwrap().clone();
    assert_eq!(entries.len(), 100 + 3 * 100 + 100);
    for index in 0..100 {
        assert_eq!(entries[index], format!("i {}", names[index]));
        assert_eq!(
            entries[entries.len() - 1 - index],
            format!("s {}", names[index])
        );
    }
}

/// 10 000 entités mutées et propagées à chaque frame, plus une chaîne
/// hiérarchique de 100 : trente frames stables, comptes exacts.
#[test]
fn ten_thousand_entities_propagate_every_frame() {
    struct Mover;
    impl System for Mover {
        fn name(&self) -> &str {
            "stress.mover"
        }
        fn run(&self, world: &mut World) -> ChaosResult<()> {
            for (_, transform) in world.query_mut::<Transform>() {
                transform.translation.x += 0.001;
            }
            Ok(())
        }
    }

    struct Builder;
    impl Subsystem for Builder {
        fn name(&self) -> &str {
            "builder"
        }
        fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
            let world = context.world_mut();
            for index in 0..10_000 {
                let entity = world.spawn()?;
                world.insert(
                    entity,
                    Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                )?;
            }
            let mut parent = world.spawn()?;
            world.insert(parent, Transform::from_translation(Vec3::ZERO))?;
            for _ in 0..99 {
                let child = world.spawn()?;
                world.insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))?;
                chaos_engine::hierarchy::attach(world, child, parent)?;
                parent = child;
            }
            context.schedule_mut().add_system(stages::UPDATE, Mover)
        }
    }

    let mut engine = Engine::new(headless(30));
    engine.add_subsystem(Box::new(Builder));
    assert_eq!(engine.run(), Ok(()));
    let snapshot = engine.metrics().snapshot();
    assert_eq!(snapshot.frame_index, 30);
    assert_eq!(snapshot.entities, 10_100);
    assert_eq!(snapshot.errors, 0);
}

/// 10 000 messages par frame pendant 50 frames : le consommateur en voit
/// EXACTEMENT 10 000 à chaque frame — jamais un cumul.
#[test]
fn a_message_flood_never_accumulates() {
    #[derive(Debug)]
    struct Ping;
    impl Message for Ping {}

    struct Producer;
    impl Subsystem for Producer {
        fn name(&self) -> &str {
            "producer"
        }
        fn update(&mut self, context: &mut EngineContext) {
            let world = context.world_mut();
            for _ in 0..10_000 {
                world.send_message(Ping);
            }
        }
    }

    struct Watcher {
        seen: Arc<Mutex<Vec<usize>>>,
    }
    impl Subsystem for Watcher {
        fn name(&self) -> &str {
            "watcher"
        }
        fn update(&mut self, context: &mut EngineContext) {
            let count = context.world().messages::<Ping>().count();
            self.seen.lock().unwrap().push(count);
        }
    }

    let seen: Arc<Mutex<Vec<usize>>> = Arc::default();
    let mut engine = Engine::new(headless(50));
    engine.add_subsystem(Box::new(Producer));
    engine.add_subsystem(Box::new(Watcher { seen: seen.clone() }));
    assert_eq!(engine.run(), Ok(()));
    let counts = seen.lock().unwrap().clone();
    assert_eq!(counts.len(), 50);
    assert!(counts.iter().all(|&count| count == 10_000));
}

/// Une scène créée, chargée, activée, désactivée et déchargée à CHAQUE
/// frame pendant 100 frames : zéro résidu, zéro dérive.
#[test]
fn scene_churn_survives_a_hundred_cycles() {
    struct Churner;
    impl Subsystem for Churner {
        fn name(&self) -> &str {
            "churner"
        }
        fn update(&mut self, context: &mut EngineContext) {
            let frame = context.time().frame_index;
            let name = format!("scenes/churn{frame}");
            let (world, scenes) = context.world_and_scenes();
            let id = scenes.create(&name).unwrap();
            scenes
                .load(world, id, |scene, world| {
                    scene.spawn(world)?;
                    scene.spawn(world)?;
                    Ok(())
                })
                .unwrap();
            scenes.activate(id).unwrap();
            scenes.deactivate(id).unwrap();
            scenes.unload(world, id).unwrap();
        }
    }

    let mut engine = Engine::new(headless(100));
    engine.add_subsystem(Box::new(Churner));
    assert_eq!(engine.run(), Ok(()));
    let snapshot = engine.metrics().snapshot();
    assert_eq!(snapshot.frame_index, 100);
    assert_eq!(snapshot.entities, 0);
    assert_eq!(snapshot.active_scenes, 0);
    assert_eq!(snapshot.errors, 0);
}
