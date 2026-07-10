use std::fmt;

use chaos_core::{ChaosError, ChaosResult};

use crate::world::World;

/// Un système : un TRAITEMENT appliqué aux composants — jamais un
/// propriétaire de données. Le `&self` de `run` est la loi de la spec par
/// construction : un système ne peut rien accumuler ; son état, s'il en
/// faut un, est une ressource du World (inspectable), jamais un champ
/// privé. Il ne reçoit que le monde et ne fait que le transformer.
/// `name()` est explicite et obligatoire : diagnostics, erreurs, futur
/// éditeur — explicite plutôt que magique.
pub trait System: Send + Sync {
    fn name(&self) -> &str;

    fn run(&self, world: &mut World) -> ChaosResult<()>;
}

/// Le registre ordonné des systèmes — la famille est complète : Entities,
/// Resources, Systems. Il vit HORS du World : les traitements ne vivent
/// pas dans les données. Exécution séquentielle dans l'ordre
/// d'enregistrement, déterministe ; le parallélisme futur viendra des
/// contraintes déjà posées (`Send + Sync` partout), pas d'une machinerie
/// anticipée. Pas de journalisation : `run` est LE chemin chaud.
#[derive(Default)]
pub struct Systems {
    systems: Vec<Box<dyn System>>,
}

impl Systems {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre un système en fin d'ordre. Un doublon de nom est rejeté
    /// en nommant l'existant — l'enregistrement est un chemin froid, le
    /// scan est honnête.
    pub fn add(&mut self, system: impl System + 'static) -> ChaosResult<()> {
        if self
            .systems
            .iter()
            .any(|existing| existing.name() == system.name())
        {
            return Err(ChaosError::Ecs(format!(
                "a system named '{}' is already registered",
                system.name()
            )));
        }
        self.systems.push(Box::new(system));
        Ok(())
    }

    /// Exécute tous les systèmes dans l'ordre d'enregistrement. La
    /// première erreur arrête le tick — un monde en état inconnu ne doit
    /// pas continuer — et nomme le système fautif. Les mutations faites
    /// avant l'échec restent : pas de rollback magique.
    pub fn run(&self, world: &mut World) -> ChaosResult<()> {
        for system in &self.systems {
            system.run(world).map_err(|error| {
                ChaosError::Ecs(format!("system '{}' failed: {error}", system.name()))
            })?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.systems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
}

impl fmt::Debug for Systems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Systems")
            .field(
                "order",
                &self
                    .systems
                    .iter()
                    .map(|system| system.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chaos_core::math::Vec3;
    use chaos_core::{Entity, Time, Transform};

    use crate::resource::Resource;

    use super::*;

    #[derive(Debug, PartialEq, Default)]
    struct Trace(Vec<&'static str>);

    impl Resource for Trace {}

    #[derive(Debug, PartialEq, Default)]
    struct Counter(u32);

    impl Resource for Counter {}

    struct Push(&'static str);

    impl System for Push {
        fn name(&self) -> &str {
            self.0
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            world.resource_mut::<Trace>().unwrap().0.push(self.0);
            Ok(())
        }
    }

    struct Advance;

    impl System for Advance {
        fn name(&self) -> &str {
            "advance"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            let delta = world.resource::<Time>().unwrap().delta_seconds();
            for (_, transform) in world.query_mut::<Transform>() {
                transform.translation.x += delta;
            }
            Ok(())
        }
    }

    struct Increment;

    impl System for Increment {
        fn name(&self) -> &str {
            "increment"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            world.resource_mut::<Counter>().unwrap().0 += 1;
            Ok(())
        }
    }

    struct Fail;

    impl System for Fail {
        fn name(&self) -> &str {
            "fail"
        }

        fn run(&self, _world: &mut World) -> ChaosResult<()> {
            Err(ChaosError::Ecs(String::from("boom")))
        }
    }

    struct SpawnOne;

    impl System for SpawnOne {
        fn name(&self) -> &str {
            "spawn_one"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            world.spawn().map(|_| ())
        }
    }

    struct CullAll;

    impl System for CullAll {
        fn name(&self) -> &str {
            "cull_all"
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            let entities: Vec<Entity> = world.iter().collect();
            for entity in entities {
                world.despawn(entity)?;
            }
            Ok(())
        }
    }

    #[test]
    fn a_new_registry_is_empty() {
        let systems = Systems::new();
        assert!(systems.is_empty());
        assert_eq!(systems.len(), 0);
    }

    #[test]
    fn a_duplicate_system_name_is_an_explicit_error() {
        let mut systems = Systems::new();
        systems.add(Push("trace")).unwrap();
        let error = systems.add(Push("trace")).unwrap_err();
        assert!(error.to_string().contains("'trace'"));
        assert!(error.to_string().contains("already registered"));
        assert_eq!(systems.len(), 1);
    }

    #[test]
    fn running_an_empty_registry_is_a_no_op() {
        let systems = Systems::new();
        let mut world = World::new();
        systems.run(&mut world).unwrap();
        assert!(world.is_empty());
    }

    #[test]
    fn run_executes_in_registration_order() {
        let mut systems = Systems::new();
        systems.add(Push("first")).unwrap();
        systems.add(Push("second")).unwrap();
        systems.add(Push("third")).unwrap();
        let mut world = World::new();
        world.insert_resource(Trace::default());
        systems.run(&mut world).unwrap();
        assert_eq!(
            world.resource::<Trace>().unwrap().0,
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn a_system_transforms_components() {
        let mut world = World::new();
        world.insert_resource(Time {
            delta: Duration::from_millis(500),
            elapsed: Duration::from_millis(500),
            frame_index: 1,
        });
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        world.insert(first, Transform::IDENTITY).unwrap();
        world
            .insert(
                second,
                Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            )
            .unwrap();
        let mut systems = Systems::new();
        systems.add(Advance).unwrap();
        systems.run(&mut world).unwrap();
        assert_eq!(world.get::<Transform>(first).unwrap().translation.x, 0.5);
        assert_eq!(world.get::<Transform>(second).unwrap().translation.x, 1.5);
    }

    #[test]
    fn a_system_reads_and_writes_resources() {
        let mut world = World::new();
        world.insert_resource(Counter::default());
        let mut systems = Systems::new();
        systems.add(Increment).unwrap();
        systems.run(&mut world).unwrap();
        systems.run(&mut world).unwrap();
        assert_eq!(world.resource::<Counter>(), Some(&Counter(2)));
    }

    #[test]
    fn a_failing_system_stops_the_tick_and_names_itself() {
        let mut systems = Systems::new();
        systems.add(Push("before")).unwrap();
        systems.add(Fail).unwrap();
        systems.add(Push("after")).unwrap();
        let mut world = World::new();
        world.insert_resource(Trace::default());
        let error = systems.run(&mut world).unwrap_err();
        assert!(error.to_string().contains("system 'fail' failed"));
        assert!(error.to_string().contains("boom"));
        assert_eq!(world.resource::<Trace>().unwrap().0, vec!["before"]);
    }

    #[test]
    fn a_system_transforms_the_structure_of_the_world() {
        let mut world = World::new();
        let mut growth = Systems::new();
        growth.add(SpawnOne).unwrap();
        growth.run(&mut world).unwrap();
        growth.run(&mut world).unwrap();
        assert_eq!(world.len(), 2);
        let mut cull = Systems::new();
        cull.add(CullAll).unwrap();
        cull.run(&mut world).unwrap();
        assert!(world.is_empty());
    }

    #[test]
    fn the_registry_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Systems>();
    }
}
