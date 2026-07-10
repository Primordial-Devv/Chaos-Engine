use std::fmt;

use chaos_core::{ChaosError, ChaosResult, Entity};

use crate::component::Component;
use crate::resource::Resource;
use crate::world::World;

type Command = Box<dyn FnOnce(&mut World) -> ChaosResult<()> + Send + Sync>;

/// Les modifications du monde, différées : enregistrées pendant qu'une
/// requête emprunte le monde, appliquées à un point sûr. La forme closure
/// dissout la référence en avant — un spawn différé compose librement
/// (spawn puis insert) dans le même différé.
///
/// Les Commands vivent HORS du World (sinon impossible d'enregistrer
/// pendant l'itération). Le patron local : buffer local pendant la
/// requête, `apply` après. Le patron cross-système : la ressource
/// `Commands` (un système tôt enregistre, un système « flush » tardif
/// applique) — les points d'application aux frontières de stages viendront
/// avec le parallélisme. Pas de journalisation : chemins chauds.
#[derive(Default)]
pub struct Commands {
    queue: Vec<Command>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre une modification arbitraire du monde.
    pub fn push(
        &mut self,
        command: impl FnOnce(&mut World) -> ChaosResult<()> + Send + Sync + 'static,
    ) {
        self.queue.push(Box::new(command));
    }

    /// Despawn différé — les règles du World s'appliqueront telles quelles
    /// à l'apply (un mort = la même erreur explicite qu'en direct).
    pub fn despawn(&mut self, entity: Entity) {
        self.push(move |world| world.despawn(entity));
    }

    /// Insert différé. La valeur remplacée est abandonnée : le différé ne
    /// peut la rendre à personne — l'appel direct la rend, lui.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) {
        self.push(move |world| world.insert(entity, value).map(|_| ()));
    }

    /// Remove différé — composant absent ou entité périmée : sans effet,
    /// comme en direct.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.push(move |world| {
            world.remove::<T>(entity);
            Ok(())
        });
    }

    /// Applique la file en FIFO, strictement : la première erreur arrête
    /// tout en nommant l'index de la commande fautive ; les mutations
    /// antérieures restent (pas de rollback magique) et la file est
    /// CONSOMMÉE dans tous les cas — jamais de rejeu à moitié. La
    /// tolérance, si voulue, se compose dans la closure de l'appelant.
    pub fn apply(&mut self, world: &mut World) -> ChaosResult<()> {
        for (index, command) in std::mem::take(&mut self.queue).into_iter().enumerate() {
            command(world).map_err(|error| {
                ChaosError::Ecs(format!("deferred command #{index} failed: {error}"))
            })?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Resource for Commands {}

impl fmt::Debug for Commands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Commands")
            .field("queued", &self.queue.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::system::{System, Systems};

    use super::*;

    #[derive(Debug, PartialEq)]
    struct Value(u32);

    impl Component for Value {}

    #[test]
    fn a_new_buffer_is_empty_and_applying_it_is_a_no_op() {
        let mut commands = Commands::new();
        assert!(commands.is_empty());
        assert_eq!(commands.len(), 0);
        let mut world = World::new();
        commands.apply(&mut world).unwrap();
        assert!(world.is_empty());
    }

    #[test]
    fn recording_changes_nothing_until_apply() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        let mut commands = Commands::new();
        commands.insert(entity, Value(1));
        assert_eq!(commands.len(), 1);
        assert_eq!(world.get::<Value>(entity), None);
        commands.apply(&mut world).unwrap();
        assert_eq!(world.get::<Value>(entity), Some(&Value(1)));
    }

    #[test]
    fn apply_runs_in_fifo_order() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        let mut commands = Commands::new();
        commands.insert(entity, Value(1));
        commands.push(move |world| {
            world
                .get_mut::<Value>(entity)
                .map(|value| value.0 *= 10)
                .ok_or_else(|| ChaosError::Ecs(String::from("value missing")))
        });
        commands.push(move |world| {
            world
                .get_mut::<Value>(entity)
                .map(|value| value.0 += 5)
                .ok_or_else(|| ChaosError::Ecs(String::from("value missing")))
        });
        commands.apply(&mut world).unwrap();
        assert_eq!(world.get::<Value>(entity), Some(&Value(15)));
    }

    #[test]
    fn despawns_recorded_during_a_query_apply_cleanly() {
        let mut world = World::new();
        for health in [0u32, 5, 0, 7] {
            let entity = world.spawn().unwrap();
            world.insert(entity, Value(health)).unwrap();
        }
        let mut commands = Commands::new();
        for (entity, value) in world.query::<Value>() {
            if value.0 == 0 {
                commands.despawn(entity);
            }
        }
        commands.apply(&mut world).unwrap();
        assert_eq!(world.len(), 2);
        assert_eq!(world.query::<Value>().count(), 2);
        assert!(world.query::<Value>().all(|(_, value)| value.0 != 0));
    }

    #[test]
    fn the_sugars_insert_remove_despawn_work() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        let mut commands = Commands::new();
        commands.insert(entity, Value(1));
        commands.remove::<Value>(entity);
        commands.apply(&mut world).unwrap();
        assert_eq!(world.get::<Value>(entity), None);
        assert!(world.is_alive(entity));
        let mut commands = Commands::new();
        commands.despawn(entity);
        commands.apply(&mut world).unwrap();
        assert!(!world.is_alive(entity));
    }

    #[test]
    fn a_failing_command_stops_strictly_and_names_its_index() {
        let mut world = World::new();
        let alive = world.spawn().unwrap();
        let forged = Entity::from_raw(99, 0);
        let mut commands = Commands::new();
        commands.insert(alive, Value(1));
        commands.despawn(forged);
        commands.insert(alive, Value(2));
        let error = commands.apply(&mut world).unwrap_err();
        assert!(error.to_string().contains("deferred command #1 failed"));
        assert!(error.to_string().contains("dead, stale or unknown"));
        assert_eq!(world.get::<Value>(alive), Some(&Value(1)));
        assert!(commands.is_empty());
    }

    #[test]
    fn the_cross_system_flush_pattern_works_end_to_end() {
        struct Recorder;

        impl System for Recorder {
            fn name(&self) -> &str {
                "recorder"
            }

            fn run(&self, world: &mut World) -> ChaosResult<()> {
                let Some(commands) = world.resource_mut::<Commands>() else {
                    return Err(ChaosError::Ecs(String::from("commands resource missing")));
                };
                commands.push(|world| world.spawn().map(|_| ()));
                Ok(())
            }
        }

        struct Flush;

        impl System for Flush {
            fn name(&self) -> &str {
                "flush"
            }

            fn run(&self, world: &mut World) -> ChaosResult<()> {
                let Some(mut commands) = world.remove_resource::<Commands>() else {
                    return Err(ChaosError::Ecs(String::from("commands resource missing")));
                };
                let outcome = commands.apply(world);
                world.insert_resource(commands);
                outcome
            }
        }

        let mut world = World::new();
        world.insert_resource(Commands::new());
        let mut systems = Systems::new();
        systems.add(Recorder).unwrap();
        systems.add(Flush).unwrap();
        systems.run(&mut world).unwrap();
        assert_eq!(world.len(), 1);
        assert!(world.resource::<Commands>().unwrap().is_empty());
    }

    #[test]
    fn the_buffer_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Commands>();
    }
}
