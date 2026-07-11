#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Pool générationnel de ressources GPU : les slots sont réutilisés mais
/// chaque réutilisation incrémente la génération — un handle périmé est
/// toujours détecté, jamais résolu vers une autre ressource.
#[derive(Debug)]
pub(crate) struct ResourcePool<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> ResourcePool<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, value: T) -> Option<PoolHandle> {
        if let Some(index) = self.free.pop() {
            let slot = self.slots.get_mut(index as usize)?;
            slot.value = Some(value);
            return Some(PoolHandle {
                index,
                generation: slot.generation,
            });
        }
        let index = u32::try_from(self.slots.len()).ok()?;
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Some(PoolHandle {
            index,
            generation: 0,
        })
    }

    pub(crate) fn get(&self, handle: PoolHandle) -> Option<&T> {
        self.slots
            .get(handle.index as usize)
            .filter(|slot| slot.generation == handle.generation)
            .and_then(|slot| slot.value.as_ref())
    }

    pub(crate) fn get_mut(&mut self, handle: PoolHandle) -> Option<&mut T> {
        self.slots
            .get_mut(handle.index as usize)
            .filter(|slot| slot.generation == handle.generation)
            .and_then(|slot| slot.value.as_mut())
    }

    pub(crate) fn remove(&mut self, handle: PoolHandle) -> Option<T> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation || slot.value.is_none() {
            return None;
        }
        let value = slot.value.take();
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.index);
        value
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get() {
        let mut pool = ResourcePool::new();
        let handle = pool.insert(String::from("a")).unwrap();
        assert_eq!(pool.get(handle), Some(&String::from("a")));
    }

    #[test]
    fn remove_then_get_returns_none() {
        let mut pool = ResourcePool::new();
        let handle = pool.insert(String::from("a")).unwrap();
        assert_eq!(pool.remove(handle), Some(String::from("a")));
        assert_eq!(pool.get(handle), None);
        assert_eq!(pool.remove(handle), None);
    }

    #[test]
    fn stale_handle_is_rejected_after_slot_reuse() {
        let mut pool = ResourcePool::new();
        let old = pool.insert(String::from("a")).unwrap();
        pool.remove(old).unwrap();
        let new = pool.insert(String::from("b")).unwrap();
        assert_eq!(new.index, old.index);
        assert_ne!(new.generation, old.generation);
        assert_eq!(pool.get(old), None);
        assert_eq!(pool.get(new), Some(&String::from("b")));
        assert_eq!(pool.remove(old), None);
    }

    #[test]
    fn fresh_slots_are_used_when_no_free_slot() {
        let mut pool = ResourcePool::new();
        let first = pool.insert(String::from("a")).unwrap();
        let second = pool.insert(String::from("b")).unwrap();
        assert_ne!(first.index, second.index);
        assert_eq!(pool.get(first), Some(&String::from("a")));
        assert_eq!(pool.get(second), Some(&String::from("b")));
    }
}
