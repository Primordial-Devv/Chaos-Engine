//! Les requêtes : demander au monde uniquement les composants dont on a
//! besoin. Rien n'est inventé ici — les requêtes EXPOSENT l'itération
//! dense que le sparse set porte depuis la sous-phase 2, plus la jointure
//! par sondage O(1). Le coût suit les correspondances, jamais la taille du
//! monde. Zéro unsafe : le split mutable vient de
//! `HashMap::get_disjoint_mut` (std). Pas de journalisation : LE chemin
//! chaud du moteur.

use chaos_core::{ChaosResult, Entity};

use crate::component::Component;
use crate::world::World;

impl World {
    /// Toutes les paires (Entity, &A) — l'itération dense du storage,
    /// coût O(|A|), indépendant du nombre total d'entités. Storage absent
    /// → itérateur vide.
    pub fn query<A: Component>(&self) -> impl Iterator<Item = (Entity, &A)> {
        self.storage::<A>()
            .into_iter()
            .flat_map(|storage| storage.iter())
    }

    /// Toutes les paires (Entity, &mut A) — la variante mutable de
    /// [`World::query`].
    pub fn query_mut<A: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut A)> {
        self.storage_mut::<A>()
            .into_iter()
            .flat_map(|storage| storage.iter_mut())
    }

    /// Jointure en lecture : (Entity, &A, &B) pour les entités portant les
    /// deux. **A mène** (itéré densément), B est sondé en O(1)
    /// générationnel — coût O(|A|) : mettez le composant le plus rare en
    /// premier. Les jointures plus larges se composent : sonder un C via
    /// `world.get::<C>(entity)` dans la boucle (emprunts partagés
    /// simultanés).
    pub fn query2<A: Component, B: Component>(&self) -> impl Iterator<Item = (Entity, &A, &B)> {
        let probe = self.storage::<B>();
        self.storage::<A>()
            .into_iter()
            .flat_map(|storage| storage.iter())
            .filter_map(move |(entity, lead)| {
                let value = probe?.get(entity)?;
                Some((entity, lead, value))
            })
    }

    /// Jointure mutable : (Entity, &mut A, &B) — A mené et muté, B lu.
    /// La seule requête faillible : A == B est une erreur explicite.
    pub fn query2_mut<A: Component, B: Component>(
        &mut self,
    ) -> ChaosResult<impl Iterator<Item = (Entity, &mut A, &B)>> {
        let (lead, probe) = self.storage_pair_mut::<A, B>()?;
        Ok(lead
            .into_iter()
            .flat_map(|storage| storage.iter_mut())
            .filter_map(move |(entity, value)| {
                let read = probe?.get(entity)?;
                Some((entity, value, read))
            }))
    }
}

#[cfg(test)]
mod tests {
    use crate::system::{System, Systems};

    use super::*;

    #[derive(Debug, PartialEq)]
    struct Value(u32);

    impl Component for Value {}

    #[derive(Debug, PartialEq)]
    struct Delta(u32);

    impl Component for Delta {}

    #[derive(Debug, PartialEq)]
    struct Tag(&'static str);

    impl Component for Tag {}

    #[test]
    fn a_query_visits_only_the_matching_entities() {
        let mut world = World::new();
        let entities: Vec<Entity> = (0..10_000).map(|_| world.spawn().unwrap()).collect();
        world.insert(entities[7], Value(1)).unwrap();
        world.insert(entities[5_000], Value(2)).unwrap();
        world.insert(entities[9_999], Value(3)).unwrap();
        let matches: Vec<(Entity, u32)> = world
            .query::<Value>()
            .map(|(entity, value)| (entity, value.0))
            .collect();
        assert_eq!(matches.len(), 3);
        assert!(matches.contains(&(entities[7], 1)));
        assert!(matches.contains(&(entities[5_000], 2)));
        assert!(matches.contains(&(entities[9_999], 3)));
    }

    #[test]
    fn a_missing_storage_yields_an_empty_query() {
        let world = World::new();
        assert_eq!(world.query::<Value>().count(), 0);
        assert_eq!(world.query2::<Value, Delta>().count(), 0);
    }

    #[test]
    fn query_mut_mutates_every_match() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        world.insert(first, Value(1)).unwrap();
        world.insert(second, Value(2)).unwrap();
        for (_, value) in world.query_mut::<Value>() {
            value.0 += 10;
        }
        assert_eq!(world.get::<Value>(first), Some(&Value(11)));
        assert_eq!(world.get::<Value>(second), Some(&Value(12)));
    }

    #[test]
    fn despawn_keeps_queries_clean() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        let third = world.spawn().unwrap();
        world.insert(first, Value(1)).unwrap();
        world.insert(second, Value(2)).unwrap();
        world.insert(third, Value(3)).unwrap();
        world.despawn(second).unwrap();
        let matches: Vec<Entity> = world.query::<Value>().map(|(entity, _)| entity).collect();
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&first));
        assert!(matches.contains(&third));
    }

    #[test]
    fn query2_joins_only_entities_having_both_components() {
        let mut world = World::new();
        let lead_only = world.spawn().unwrap();
        let probe_only = world.spawn().unwrap();
        let both_a = world.spawn().unwrap();
        let both_b = world.spawn().unwrap();
        world.insert(lead_only, Value(1)).unwrap();
        world.insert(probe_only, Delta(10)).unwrap();
        world.insert(both_a, Value(2)).unwrap();
        world.insert(both_a, Delta(20)).unwrap();
        world.insert(both_b, Value(3)).unwrap();
        world.insert(both_b, Delta(30)).unwrap();
        let pairs: Vec<(Entity, u32, u32)> = world
            .query2::<Value, Delta>()
            .map(|(entity, value, delta)| (entity, value.0, delta.0))
            .collect();
        assert_eq!(pairs.len(), 2);
        assert!(pairs.contains(&(both_a, 2, 20)));
        assert!(pairs.contains(&(both_b, 3, 30)));
    }

    #[test]
    fn query2_with_a_missing_probe_storage_is_empty() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Value(1)).unwrap();
        assert_eq!(world.query2::<Value, Delta>().count(), 0);
    }

    #[test]
    fn query2_mut_mutates_the_lead_using_the_probe() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        world.insert(first, Value(1)).unwrap();
        world.insert(first, Delta(10)).unwrap();
        world.insert(second, Value(2)).unwrap();
        world.insert(second, Delta(20)).unwrap();
        for (_, value, delta) in world.query2_mut::<Value, Delta>().unwrap() {
            value.0 += delta.0;
        }
        assert_eq!(world.get::<Value>(first), Some(&Value(11)));
        assert_eq!(world.get::<Value>(second), Some(&Value(22)));
    }

    #[test]
    fn query2_mut_skips_entities_missing_the_probe() {
        let mut world = World::new();
        let joined = world.spawn().unwrap();
        let alone = world.spawn().unwrap();
        world.insert(joined, Value(1)).unwrap();
        world.insert(joined, Delta(10)).unwrap();
        world.insert(alone, Value(5)).unwrap();
        let mut visited = 0;
        for (_, value, delta) in world.query2_mut::<Value, Delta>().unwrap() {
            value.0 += delta.0;
            visited += 1;
        }
        assert_eq!(visited, 1);
        assert_eq!(world.get::<Value>(joined), Some(&Value(11)));
        assert_eq!(world.get::<Value>(alone), Some(&Value(5)));
    }

    #[test]
    fn a_mutable_query_against_the_same_type_is_an_explicit_error() {
        let mut world = World::new();
        let error = world.query2_mut::<Value, Value>().map(|_| ()).unwrap_err();
        assert!(error.to_string().contains("Value"));
        assert!(error.to_string().contains("against itself"));
    }

    #[test]
    fn read_queries_compose_for_wider_joins() {
        let mut world = World::new();
        let full = world.spawn().unwrap();
        let partial = world.spawn().unwrap();
        world.insert(full, Value(1)).unwrap();
        world.insert(full, Delta(10)).unwrap();
        world.insert(full, Tag("full")).unwrap();
        world.insert(partial, Value(2)).unwrap();
        world.insert(partial, Delta(20)).unwrap();
        let triples: Vec<(u32, u32, &str)> = world
            .query2::<Value, Delta>()
            .filter_map(|(entity, value, delta)| {
                world
                    .get::<Tag>(entity)
                    .map(|tag| (value.0, delta.0, tag.0))
            })
            .collect();
        assert_eq!(triples, vec![(1, 10, "full")]);
    }

    #[test]
    fn a_system_body_runs_on_queries() {
        struct Integrate;

        impl System for Integrate {
            fn name(&self) -> &str {
                "integrate"
            }

            fn run(&self, world: &mut World) -> ChaosResult<()> {
                for (_, value, delta) in world.query2_mut::<Value, Delta>()? {
                    value.0 += delta.0;
                }
                Ok(())
            }
        }

        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Value(1)).unwrap();
        world.insert(entity, Delta(10)).unwrap();
        let mut systems = Systems::new();
        systems.add(Integrate).unwrap();
        systems.run(&mut world).unwrap();
        systems.run(&mut world).unwrap();
        assert_eq!(world.get::<Value>(entity), Some(&Value(21)));
    }
}
