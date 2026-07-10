use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use chaos_core::{FixedTime, Time};

/// Une ressource : des DONNÉES globales du monde qui n'appartiennent à
/// aucune entité — temps global, paramètres moteur, configuration, état
/// global. Le miroir exact de `Component` : opt-in explicite (implémenter
/// documente l'intention), jamais de comportement, `Send + Sync + 'static`
/// pour le parallélisme futur par contrainte.
pub trait Resource: Send + Sync + 'static {}

impl Resource for Time {}

impl Resource for FixedTime {}

/// Le registre des ressources : au plus UNE valeur par type (clé `TypeId`).
/// Le même mécanisme type-erased que les storages du World, sans la
/// dimension entité — aucune opération ne traverse tous les types, le
/// `Box<dyn Any + Send + Sync>` nu suffit. Pas de journalisation :
/// chemins chauds.
#[derive(Default)]
pub struct Resources {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dépose (ou remplace) la ressource du type et rend la valeur
    /// remplacée — jamais de drop silencieux.
    pub fn insert<T: Resource>(&mut self, value: T) -> Option<T> {
        let previous = self.values.insert(TypeId::of::<T>(), Box::new(value))?;
        previous.downcast::<T>().ok().map(|boxed| *boxed)
    }

    pub fn get<T: Resource>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    pub fn get_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.values.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()
    }

    /// Retire la ressource et rend sa valeur. La branche impossible par
    /// construction (la clé `TypeId` garantit le type) réinsère la boîte
    /// au lieu de la détruire — rien n'est jamais perdu.
    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        let boxed = self.values.remove(&TypeId::of::<T>())?;
        match boxed.downcast::<T>() {
            Ok(value) => Some(*value),
            Err(boxed) => {
                self.values.insert(TypeId::of::<T>(), boxed);
                None
            }
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl fmt::Debug for Resources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resources")
            .field("types", &self.values.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[derive(Debug, PartialEq)]
    struct TickRate(u32);

    impl Resource for TickRate {}

    #[derive(Debug, PartialEq)]
    struct AppName(&'static str);

    impl Resource for AppName {}

    #[test]
    fn a_new_registry_is_empty() {
        let resources = Resources::new();
        assert!(resources.is_empty());
        assert_eq!(resources.len(), 0);
        assert_eq!(resources.get::<TickRate>(), None);
    }

    #[test]
    fn insert_then_get_roundtrips_a_real_resource() {
        let mut resources = Resources::new();
        let time = Time {
            delta: Duration::from_millis(16),
            elapsed: Duration::from_millis(160),
            frame_index: 10,
            ..Time::default()
        };
        assert_eq!(resources.insert(time), None);
        assert_eq!(resources.get::<Time>(), Some(&time));
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn insert_replaces_and_returns_the_previous_value() {
        let mut resources = Resources::new();
        assert_eq!(resources.insert(TickRate(60)), None);
        assert_eq!(resources.insert(TickRate(144)), Some(TickRate(60)));
        assert_eq!(resources.get::<TickRate>(), Some(&TickRate(144)));
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn get_mut_mutates_in_place() {
        let mut resources = Resources::new();
        resources.insert(TickRate(30));
        resources.get_mut::<TickRate>().unwrap().0 = 120;
        assert_eq!(resources.get::<TickRate>(), Some(&TickRate(120)));
    }

    #[test]
    fn remove_returns_the_value_then_nothing() {
        let mut resources = Resources::new();
        resources.insert(AppName("chaos"));
        assert_eq!(resources.remove::<AppName>(), Some(AppName("chaos")));
        assert_eq!(resources.get::<AppName>(), None);
        assert_eq!(resources.remove::<AppName>(), None);
        assert!(resources.is_empty());
    }

    #[test]
    fn multiple_resource_types_coexist_independently() {
        let mut resources = Resources::new();
        resources.insert(TickRate(60));
        resources.insert(AppName("chaos"));
        resources.insert(Time::default());
        assert_eq!(resources.len(), 3);
        resources.get_mut::<TickRate>().unwrap().0 = 90;
        assert_eq!(resources.get::<TickRate>(), Some(&TickRate(90)));
        assert_eq!(resources.get::<AppName>(), Some(&AppName("chaos")));
        assert_eq!(resources.get::<Time>(), Some(&Time::default()));
    }

    #[test]
    fn the_registry_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Resources>();
    }
}
