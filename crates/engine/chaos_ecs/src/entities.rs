use chaos_core::{ChaosError, ChaosResult, Entity};

#[derive(Debug)]
struct Slot {
    generation: u32,
    alive: bool,
}

/// L'allocateur d'entités : la seule fabrique d'Entity valides du moteur.
/// Recyclage générationnel (le patron des pools du renderer) : un slot
/// réutilisé change de génération, une Entity détruite est détectée à
/// jamais — jamais résolue vers une autre. Aucune journalisation ici :
/// spawn/despawn sont des chemins chauds.
#[derive(Debug, Default)]
pub struct Entities {
    slots: Vec<Slot>,
    free: Vec<u32>,
    alive_count: usize,
}

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    /// Crée une entité vivante. L'échec est théorique (épuisement des
    /// index u32) mais explicite — jamais un panic.
    pub fn spawn(&mut self) -> ChaosResult<Entity> {
        if let Some(index) = self.free.pop() {
            let Some(slot) = self.slots.get_mut(index as usize) else {
                return Err(ChaosError::Ecs(String::from(
                    "entity allocator free list is corrupted",
                )));
            };
            slot.alive = true;
            self.alive_count += 1;
            return Ok(Entity::from_raw(index, slot.generation));
        }
        let index = u32::try_from(self.slots.len())
            .map_err(|_| ChaosError::Ecs(String::from("entity index space is exhausted")))?;
        self.slots.push(Slot {
            generation: 0,
            alive: true,
        });
        self.alive_count += 1;
        Ok(Entity::from_raw(index, 0))
    }

    /// Détruit une entité vivante ; une entité morte, périmée ou forgée est
    /// une erreur explicite. Le slot change de génération et redevient
    /// disponible.
    pub fn despawn(&mut self, entity: Entity) -> ChaosResult<()> {
        match self.slots.get_mut(entity.index() as usize) {
            Some(slot) if slot.alive && slot.generation == entity.generation() => {
                slot.alive = false;
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(entity.index());
                self.alive_count -= 1;
                Ok(())
            }
            _ => Err(ChaosError::Ecs(format!(
                "cannot despawn {entity}: dead, stale or unknown"
            ))),
        }
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.slots
            .get(entity.index() as usize)
            .is_some_and(|slot| slot.alive && slot.generation == entity.generation())
    }

    pub fn len(&self) -> usize {
        self.alive_count
    }

    pub fn is_empty(&self) -> bool {
        self.alive_count == 0
    }

    /// Les entités vivantes — le socle des futures requêtes, du debug et
    /// de l'éditeur.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.alive)
            .map(|(index, slot)| Entity::from_raw(index as u32, slot.generation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_creates_distinct_alive_entities() {
        let mut entities = Entities::new();
        let first = entities.spawn().unwrap();
        let second = entities.spawn().unwrap();
        assert_ne!(first, second);
        assert!(entities.is_alive(first));
        assert!(entities.is_alive(second));
        assert_eq!(entities.len(), 2);
    }

    #[test]
    fn despawn_kills_the_entity() {
        let mut entities = Entities::new();
        let entity = entities.spawn().unwrap();
        entities.despawn(entity).unwrap();
        assert!(!entities.is_alive(entity));
        assert!(entities.is_empty());
    }

    #[test]
    fn recycled_slots_get_a_fresh_generation() {
        let mut entities = Entities::new();
        let old = entities.spawn().unwrap();
        entities.despawn(old).unwrap();
        let new = entities.spawn().unwrap();
        assert_eq!(new.index(), old.index());
        assert_ne!(new.generation(), old.generation());
        assert!(!entities.is_alive(old));
        assert!(entities.is_alive(new));
    }

    #[test]
    fn despawning_a_stale_entity_is_an_error() {
        let mut entities = Entities::new();
        let old = entities.spawn().unwrap();
        entities.despawn(old).unwrap();
        entities.spawn().unwrap();
        let error = entities.despawn(old).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
    }

    #[test]
    fn double_despawn_is_an_error() {
        let mut entities = Entities::new();
        let entity = entities.spawn().unwrap();
        entities.despawn(entity).unwrap();
        assert!(entities.despawn(entity).is_err());
    }

    #[test]
    fn forged_entities_are_harmless() {
        let mut entities = Entities::new();
        entities.spawn().unwrap();
        let forged = Entity::from_raw(99, 0);
        assert!(!entities.is_alive(forged));
        assert!(entities.despawn(forged).is_err());
        let wrong_generation = Entity::from_raw(0, 42);
        assert!(!entities.is_alive(wrong_generation));
    }

    #[test]
    fn iteration_lists_only_the_living() {
        let mut entities = Entities::new();
        let first = entities.spawn().unwrap();
        let second = entities.spawn().unwrap();
        let third = entities.spawn().unwrap();
        entities.despawn(second).unwrap();
        let alive: Vec<Entity> = entities.iter().collect();
        assert_eq!(alive.len(), 2);
        assert!(alive.contains(&first));
        assert!(alive.contains(&third));
    }

    #[test]
    fn an_empty_allocator_reports_empty() {
        let entities = Entities::new();
        assert!(entities.is_empty());
        assert_eq!(entities.len(), 0);
        assert_eq!(entities.iter().count(), 0);
    }
}
