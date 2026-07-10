use std::fmt;

/// Identité d'un objet du monde — rien d'autre : pas de logique, pas de
/// comportement. Générationnelle : un index recyclé change de génération,
/// une Entity détruite ne se résout donc jamais vers une autre entité.
///
/// Les Entity valides sont fabriquées par l'allocateur de `chaos_ecs` ;
/// `from_raw` existe pour les systèmes du moteur (sérialisation, réseau) —
/// une Entity forgée est inoffensive par construction : l'allocateur la
/// rejette si elle ne correspond à rien de vivant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    pub fn from_raw(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entity:{}v{}", self.index, self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_roundtrips_through_the_accessors() {
        let entity = Entity::from_raw(7, 3);
        assert_eq!(entity.index(), 7);
        assert_eq!(entity.generation(), 3);
    }

    #[test]
    fn identity_distinguishes_index_and_generation() {
        let entity = Entity::from_raw(1, 1);
        assert_eq!(entity, Entity::from_raw(1, 1));
        assert_ne!(entity, Entity::from_raw(2, 1));
        assert_ne!(entity, Entity::from_raw(1, 2));
    }

    #[test]
    fn display_is_stable() {
        assert_eq!(Entity::from_raw(42, 5).to_string(), "entity:42v5");
    }
}
