use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use chaos_core::{ChaosError, ChaosResult, Entity};

use crate::component::Component;
use crate::entities::Entities;
use crate::message::{Message, Messages};
use crate::resource::{Resource, Resources};
use crate::storage::ComponentStorage;

/// La paire du split mutable : le meneur en exclusif, la sonde en partagé
/// — chacun absent si son type n'a jamais été stocké.
pub(crate) type StoragePairMut<'world, A, B> = (
    Option<&'world mut ComponentStorage<A>>,
    Option<&'world ComponentStorage<B>>,
);

/// Le pont type-erased du registre : détacher les composants d'une entité
/// sans connaître leur type — le seul besoin du despawn multi-types.
trait AnyStorage: Send + Sync {
    fn detach(&mut self, entity: Entity);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Component> AnyStorage for ComponentStorage<T> {
    fn detach(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Le conteneur central du monde : l'allocateur d'entités + un storage par
/// type de composant (registre par `TypeId` — le `'static` du trait
/// `Component` paie ici). Le World porte la cohérence que les briques
/// seules n'ont pas :
///
/// - **vivacité à l'écriture** : `insert` refuse une entité morte, périmée
///   ou forgée (erreur explicite) — la promesse documentée du storage est
///   tenue ici ;
/// - **pas de données orphelines** : `despawn` détache les composants de
///   tous les storages — un index recyclé démarre propre, la sûreté
///   générationnelle du storage reste la seconde ligne de défense.
///
/// Le World porte aussi les **ressources** : les données globales qui
/// n'appartiennent à aucune entité (temps, paramètres, configuration) —
/// un registre distinct des composants, au plus une valeur par type.
///
/// La doctrine des retours : lecture/détachement en `Option` (le périmé
/// est inoffensif par construction), écriture en erreur (écrire pour un
/// mort est un bug de logique, nommé). Les ressources n'ont pas de
/// vivacité : tout en `Option`, remplacer est légitime (la valeur
/// remplacée est rendue). `Send + Sync` par construction — verrouillé par
/// test. Pas de journalisation : chemins chauds.
#[derive(Default)]
pub struct World {
    entities: Entities,
    storages: HashMap<TypeId, Box<dyn AnyStorage>>,
    resources: Resources,
    messages: Messages,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Crée une entité vivante dans le monde.
    pub fn spawn(&mut self) -> ChaosResult<Entity> {
        self.entities.spawn()
    }

    /// Détruit une entité vivante et détache tous ses composants. Une
    /// entité morte, périmée ou forgée erre AVANT tout détachement :
    /// l'occupant actuel d'un index recyclé n'est jamais touché.
    pub fn despawn(&mut self, entity: Entity) -> ChaosResult<()> {
        self.entities.despawn(entity)?;
        for storage in self.storages.values_mut() {
            storage.detach(entity);
        }
        Ok(())
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Les entités vivantes du monde.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter()
    }

    /// Attache (ou remplace) un composant et rend la valeur remplacée.
    /// La garantie d'écriture du World : une entité non vivante est une
    /// erreur explicite, jamais une écriture silencieuse.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> ChaosResult<Option<T>> {
        if !self.entities.is_alive(entity) {
            return Err(ChaosError::Ecs(format!(
                "cannot insert {} on {entity}: dead, stale or unknown",
                std::any::type_name::<T>()
            )));
        }
        let storage = self
            .storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(ComponentStorage::<T>::new()));
        match storage.as_any_mut().downcast_mut::<ComponentStorage<T>>() {
            Some(storage) => Ok(storage.insert(entity, value)),
            None => Err(ChaosError::Ecs(format!(
                "component storage registry is corrupted for {}",
                std::any::type_name::<T>()
            ))),
        }
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.storage::<T>()?.get(entity)
    }

    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        self.storage_mut::<T>()?.get_mut(entity)
    }

    /// Détache un composant et rend sa valeur — `None` si l'entité ne le
    /// porte pas (ou est périmée : elle ne résout jamais).
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        self.storage_mut::<T>()?.remove(entity)
    }

    /// Dépose (ou remplace) une ressource globale et rend la valeur
    /// remplacée.
    pub fn insert_resource<T: Resource>(&mut self, value: T) -> Option<T> {
        self.resources.insert(value)
    }

    pub fn resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    pub fn resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    /// Retire une ressource globale et rend sa valeur.
    pub fn remove_resource<T: Resource>(&mut self) -> Option<T> {
        self.resources.remove::<T>()
    }

    /// Publie un message — la file de son type est auto-créée.
    pub fn send_message<T: Message>(&mut self, message: T) {
        self.messages.send(message);
    }

    /// Lit les messages du type, dans l'ordre d'émission.
    pub fn messages<T: Message>(&self) -> impl Iterator<Item = &T> {
        self.messages.read::<T>()
    }

    /// Consomme les messages du type (ordre préservé, la file reste vide).
    pub fn drain_messages<T: Message>(&mut self) -> impl Iterator<Item = T> {
        self.messages.drain::<T>()
    }

    /// Balaye TOUTES les files de messages — la primitive de la boucle
    /// moteur, une fois par frame.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    pub(crate) fn storage<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())?
            .as_any()
            .downcast_ref::<ComponentStorage<T>>()
    }

    pub(crate) fn storage_mut<T: Component>(&mut self) -> Option<&mut ComponentStorage<T>> {
        self.storages
            .get_mut(&TypeId::of::<T>())?
            .as_any_mut()
            .downcast_mut::<ComponentStorage<T>>()
    }

    /// Le split sûr de deux storages pour la jointure mutable des requêtes :
    /// deux `&mut` disjoints prouvés par les clés (`get_disjoint_mut` de
    /// std — zéro unsafe), le second rétrogradé en partagé. A == B est une
    /// erreur explicite : l'aliasing &mut/& d'un même storage est
    /// indéfendable, et jamais un panic.
    pub(crate) fn storage_pair_mut<A: Component, B: Component>(
        &mut self,
    ) -> ChaosResult<StoragePairMut<'_, A, B>> {
        if TypeId::of::<A>() == TypeId::of::<B>() {
            return Err(ChaosError::Ecs(format!(
                "cannot query '{}' mutably against itself",
                std::any::type_name::<A>()
            )));
        }
        let [lead, probe] = self
            .storages
            .get_disjoint_mut([&TypeId::of::<A>(), &TypeId::of::<B>()]);
        let lead =
            lead.and_then(|storage| storage.as_any_mut().downcast_mut::<ComponentStorage<A>>());
        let probe = probe
            .and_then(|storage| storage.as_any_mut().downcast_mut::<ComponentStorage<B>>())
            .map(|storage| &*storage);
        Ok((lead, probe))
    }
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World")
            .field("entities", &self.entities)
            .field("component_types", &self.storages.len())
            .field("resources", &self.resources.len())
            .field("messages", &self.messages)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::Transform;
    use chaos_core::math::Vec3;

    use super::*;

    #[derive(Debug, PartialEq)]
    struct Health(u32);

    impl Component for Health {}

    #[derive(Debug, PartialEq)]
    struct Label(&'static str);

    impl Component for Label {}

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    impl Component for Counter {}

    impl Resource for Counter {}

    #[test]
    fn a_new_world_is_empty() {
        let world = World::new();
        assert!(world.is_empty());
        assert_eq!(world.len(), 0);
        assert_eq!(world.iter().count(), 0);
    }

    #[test]
    fn spawn_gives_a_live_entity_and_counts_track() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        assert!(world.is_alive(entity));
        assert_eq!(world.len(), 1);
        assert!(!world.is_empty());
    }

    #[test]
    fn despawn_kills_and_double_despawn_is_an_explicit_error() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.despawn(entity).unwrap();
        assert!(!world.is_alive(entity));
        let error = world.despawn(entity).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
    }

    #[test]
    fn insert_then_get_roundtrips_a_real_component() {
        let mut world = World::new();
        let player = world.spawn().unwrap();
        world
            .insert(
                player,
                Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
            )
            .unwrap();
        let transform = world.get::<Transform>(player).unwrap();
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn inserting_on_a_dead_entity_is_an_explicit_error() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.despawn(entity).unwrap();
        let error = world.insert(entity, Health(1)).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
        assert!(error.to_string().contains("Health"));
    }

    #[test]
    fn insert_replaces_and_returns_the_previous_value() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        assert_eq!(world.insert(entity, Health(100)).unwrap(), None);
        assert_eq!(world.insert(entity, Health(50)).unwrap(), Some(Health(100)));
        assert_eq!(world.get::<Health>(entity), Some(&Health(50)));
    }

    #[test]
    fn get_mut_mutates_through_the_world() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Health(10)).unwrap();
        world.get_mut::<Health>(entity).unwrap().0 = 99;
        assert_eq!(world.get::<Health>(entity), Some(&Health(99)));
    }

    #[test]
    fn remove_returns_the_value_then_nothing() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Health(7)).unwrap();
        assert_eq!(world.remove::<Health>(entity), Some(Health(7)));
        assert_eq!(world.get::<Health>(entity), None);
        assert_eq!(world.remove::<Health>(entity), None);
        assert!(world.is_alive(entity));
    }

    #[test]
    fn despawn_detaches_every_component_of_the_entity() {
        let mut world = World::new();
        let old = world.spawn().unwrap();
        world.insert(old, Health(5)).unwrap();
        world.insert(old, Label("goblin")).unwrap();
        world.despawn(old).unwrap();
        let recycled = world.spawn().unwrap();
        assert_eq!(recycled.index(), old.index());
        assert_eq!(world.get::<Health>(recycled), None);
        assert_eq!(world.get::<Label>(recycled), None);
        assert_eq!(world.insert(recycled, Health(9)).unwrap(), None);
    }

    #[test]
    fn a_stale_entity_never_resolves_through_the_world() {
        let mut world = World::new();
        let old = world.spawn().unwrap();
        world.insert(old, Health(1)).unwrap();
        world.despawn(old).unwrap();
        let recycled = world.spawn().unwrap();
        world.insert(recycled, Health(2)).unwrap();
        assert!(!world.is_alive(old));
        assert_eq!(world.get::<Health>(old), None);
        assert_eq!(world.remove::<Health>(old), None);
        assert_eq!(world.get::<Health>(recycled), Some(&Health(2)));
    }

    #[test]
    fn a_stale_despawn_does_not_touch_the_current_occupant() {
        let mut world = World::new();
        let old = world.spawn().unwrap();
        world.despawn(old).unwrap();
        let recycled = world.spawn().unwrap();
        world.insert(recycled, Health(3)).unwrap();
        assert!(world.despawn(old).is_err());
        assert!(world.is_alive(recycled));
        assert_eq!(world.get::<Health>(recycled), Some(&Health(3)));
    }

    #[test]
    fn multiple_component_types_coexist_on_one_entity() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Transform::IDENTITY).unwrap();
        world.insert(entity, Health(12)).unwrap();
        world.insert(entity, Label("crate")).unwrap();
        assert!(world.get::<Transform>(entity).is_some());
        assert_eq!(world.get::<Health>(entity), Some(&Health(12)));
        assert_eq!(world.get::<Label>(entity), Some(&Label("crate")));
    }

    #[test]
    fn multiple_entities_hold_independent_values() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        world.insert(first, Health(1)).unwrap();
        world.insert(second, Health(2)).unwrap();
        world.get_mut::<Health>(first).unwrap().0 = 10;
        assert_eq!(world.get::<Health>(first), Some(&Health(10)));
        assert_eq!(world.get::<Health>(second), Some(&Health(2)));
    }

    #[test]
    fn iter_yields_live_entities_only() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        let third = world.spawn().unwrap();
        world.despawn(second).unwrap();
        let alive: Vec<Entity> = world.iter().collect();
        assert_eq!(alive.len(), 2);
        assert!(alive.contains(&first));
        assert!(alive.contains(&third));
    }

    #[test]
    fn the_world_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<World>();
    }

    #[test]
    fn insert_resource_then_resource_roundtrips_through_the_world() {
        let mut world = World::new();
        let time = chaos_core::Time {
            delta: std::time::Duration::from_millis(16),
            elapsed: std::time::Duration::from_millis(32),
            frame_index: 2,
        };
        assert_eq!(world.insert_resource(time), None);
        assert_eq!(world.resource::<chaos_core::Time>(), Some(&time));
    }

    #[test]
    fn insert_resource_replaces_and_returns_the_previous_value() {
        let mut world = World::new();
        assert_eq!(world.insert_resource(Counter(1)), None);
        assert_eq!(world.insert_resource(Counter(2)), Some(Counter(1)));
        assert_eq!(world.resource::<Counter>(), Some(&Counter(2)));
    }

    #[test]
    fn resource_mut_mutates_through_the_world() {
        let mut world = World::new();
        world.insert_resource(Counter(5));
        world.resource_mut::<Counter>().unwrap().0 = 50;
        assert_eq!(world.resource::<Counter>(), Some(&Counter(50)));
    }

    #[test]
    fn remove_resource_returns_the_value_then_nothing() {
        let mut world = World::new();
        world.insert_resource(Counter(3));
        assert_eq!(world.remove_resource::<Counter>(), Some(Counter(3)));
        assert_eq!(world.resource::<Counter>(), None);
        assert_eq!(world.remove_resource::<Counter>(), None);
    }

    #[test]
    fn resources_survive_the_entity_lifecycle() {
        let mut world = World::new();
        world.insert_resource(Counter(7));
        let entity = world.spawn().unwrap();
        world.insert(entity, Health(1)).unwrap();
        world.despawn(entity).unwrap();
        assert!(world.is_empty());
        assert_eq!(world.resource::<Counter>(), Some(&Counter(7)));
    }

    #[test]
    fn the_resource_registry_is_distinct_from_components() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Counter(1)).unwrap();
        world.insert_resource(Counter(2));
        assert_eq!(world.get::<Counter>(entity), Some(&Counter(1)));
        assert_eq!(world.resource::<Counter>(), Some(&Counter(2)));
        world.resource_mut::<Counter>().unwrap().0 = 20;
        assert_eq!(world.get::<Counter>(entity), Some(&Counter(1)));
        assert_eq!(world.remove::<Counter>(entity), Some(Counter(1)));
        assert_eq!(world.resource::<Counter>(), Some(&Counter(20)));
    }
}
