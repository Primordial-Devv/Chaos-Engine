use chaos_core::Entity;

use crate::component::Component;

/// Stockage d'UN type de composant — un sparse set générationnel : accès
/// O(1) par entité, données denses et contiguës pour l'itération (le socle
/// des futures requêtes). L'entrée dense porte l'Entity complète : une
/// entité périmée n'est JAMAIS résolue vers les données d'une autre.
///
/// Le storage ne connaît pas la vivacité — c'est le World qui garantit de
/// n'écrire que pour des entités vivantes ; en lecture, la sûreté
/// générationnelle est inconditionnelle. Pas de journalisation : chemins
/// chauds.
#[derive(Debug, Default)]
pub struct ComponentStorage<T: Component> {
    sparse: Vec<Option<usize>>,
    entities: Vec<Entity>,
    data: Vec<T>,
}

impl<T: Component> ComponentStorage<T> {
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            entities: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Attache (ou remplace) le composant de l'entité et rend la valeur
    /// délogée — y compris celle d'une génération antérieure du même index
    /// (jamais de drop silencieux ; le World empêche ce cas en pratique).
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        let index = entity.index() as usize;
        if index >= self.sparse.len() {
            self.sparse.resize(index + 1, None);
        }
        match self.sparse[index] {
            Some(dense) => {
                let previous = std::mem::replace(&mut self.data[dense], value);
                self.entities[dense] = entity;
                Some(previous)
            }
            None => {
                self.sparse[index] = Some(self.data.len());
                self.entities.push(entity);
                self.data.push(value);
                None
            }
        }
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        let dense = (*self.sparse.get(entity.index() as usize)?)?;
        if self.entities.get(dense) == Some(&entity) {
            self.data.get(dense)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let dense = (*self.sparse.get(entity.index() as usize)?)?;
        if self.entities.get(dense) == Some(&entity) {
            self.data.get_mut(dense)
        } else {
            None
        }
    }

    /// Détache le composant et rend sa valeur. L'invariant du sparse set :
    /// le dernier élément dense prend la place du retiré (`swap_remove`) et
    /// son entrée sparse est corrigée.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let index = entity.index() as usize;
        let dense = (*self.sparse.get(index)?)?;
        if self.entities.get(dense) != Some(&entity) {
            return None;
        }
        self.sparse[index] = None;
        let value = self.data.swap_remove(dense);
        self.entities.swap_remove(dense);
        if let Some(moved) = self.entities.get(dense) {
            self.sparse[moved.index() as usize] = Some(dense);
        }
        Some(value)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Itération dense : chaque paire (Entity, &composant).
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.entities.iter().copied().zip(self.data.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.entities.iter().copied().zip(self.data.iter_mut())
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

    fn entity(index: u32) -> Entity {
        Entity::from_raw(index, 0)
    }

    #[test]
    fn insert_then_get_roundtrips_a_real_component() {
        let mut storage = ComponentStorage::new();
        let player = entity(0);
        storage.insert(
            player,
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        );
        let transform = storage.get(player).unwrap();
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn insert_replaces_and_returns_the_previous_value() {
        let mut storage = ComponentStorage::new();
        let target = entity(0);
        assert_eq!(storage.insert(target, Health(100)), None);
        assert_eq!(storage.insert(target, Health(50)), Some(Health(100)));
        assert_eq!(storage.get(target), Some(&Health(50)));
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn get_mut_mutates_in_place() {
        let mut storage = ComponentStorage::new();
        let target = entity(3);
        storage.insert(target, Health(10));
        storage.get_mut(target).unwrap().0 = 99;
        assert_eq!(storage.get(target), Some(&Health(99)));
    }

    #[test]
    fn remove_returns_the_value_and_swap_remove_keeps_others_resolvable() {
        let mut storage = ComponentStorage::new();
        let first = entity(0);
        let second = entity(1);
        let third = entity(2);
        storage.insert(first, Health(1));
        storage.insert(second, Health(2));
        storage.insert(third, Health(3));
        assert_eq!(storage.remove(second), Some(Health(2)));
        assert_eq!(storage.get(first), Some(&Health(1)));
        assert_eq!(storage.get(third), Some(&Health(3)));
        assert!(!storage.contains(second));
        assert_eq!(storage.len(), 2);
    }

    #[test]
    fn a_stale_entity_is_never_resolved() {
        let mut storage = ComponentStorage::new();
        let old = Entity::from_raw(0, 0);
        let new = Entity::from_raw(0, 1);
        storage.insert(new, Health(7));
        assert_eq!(storage.get(old), None);
        assert!(!storage.contains(old));
        assert!(storage.get_mut(old).is_none());
        assert_eq!(storage.remove(old), None);
        assert_eq!(storage.get(new), Some(&Health(7)));
    }

    #[test]
    fn inserting_over_a_recycled_index_returns_the_displaced_value() {
        let mut storage = ComponentStorage::new();
        let old = Entity::from_raw(0, 0);
        let new = Entity::from_raw(0, 1);
        storage.insert(old, Health(1));
        assert_eq!(storage.insert(new, Health(2)), Some(Health(1)));
        assert_eq!(storage.get(new), Some(&Health(2)));
        assert_eq!(storage.get(old), None);
    }

    #[test]
    fn iteration_yields_complete_pairs() {
        let mut storage = ComponentStorage::new();
        let first = Entity::from_raw(2, 5);
        let second = Entity::from_raw(7, 1);
        storage.insert(first, Health(20));
        storage.insert(second, Health(70));
        let pairs: Vec<(Entity, u32)> = storage
            .iter()
            .map(|(entity, health)| (entity, health.0))
            .collect();
        assert_eq!(pairs.len(), 2);
        assert!(pairs.contains(&(first, 20)));
        assert!(pairs.contains(&(second, 70)));
    }

    #[test]
    fn iter_mut_mutates_every_component() {
        let mut storage = ComponentStorage::new();
        storage.insert(entity(0), Health(1));
        storage.insert(entity(1), Health(2));
        for (_, health) in storage.iter_mut() {
            health.0 += 10;
        }
        assert_eq!(storage.get(entity(0)), Some(&Health(11)));
        assert_eq!(storage.get(entity(1)), Some(&Health(12)));
    }

    #[test]
    fn an_empty_storage_reports_empty() {
        let storage: ComponentStorage<Health> = ComponentStorage::new();
        assert!(storage.is_empty());
        assert_eq!(storage.len(), 0);
        assert_eq!(storage.iter().count(), 0);
        assert!(!storage.contains(entity(0)));
    }

    #[test]
    fn sparse_entities_far_apart_coexist() {
        let mut storage = ComponentStorage::new();
        storage.insert(entity(0), Health(1));
        storage.insert(entity(1000), Health(2));
        assert_eq!(storage.get(entity(0)), Some(&Health(1)));
        assert_eq!(storage.get(entity(1000)), Some(&Health(2)));
        assert_eq!(storage.len(), 2);
    }

    #[test]
    fn removing_the_last_dense_element_is_clean() {
        let mut storage = ComponentStorage::new();
        let only = entity(4);
        storage.insert(only, Health(9));
        assert_eq!(storage.remove(only), Some(Health(9)));
        assert!(storage.is_empty());
        assert_eq!(storage.remove(only), None);
    }
}
