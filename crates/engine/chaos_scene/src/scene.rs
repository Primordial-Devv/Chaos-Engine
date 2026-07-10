use chaos_core::{ChaosError, ChaosResult, Entity, SceneId};
use chaos_ecs::World;

use crate::member::SceneMember;

/// Le modèle de cycle de vie d'une scène — le vocabulaire complet exigé
/// par la phase 5. Chaque transition arrivera AVEC sa machinerie (le
/// chargement avec la persistance, l'activation avec la gestion du cycle
/// de vie) : ici, seul l'état initial `Empty` a une fabrique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneState {
    /// Créée, aucun contenu.
    Empty,
    /// Contenu chargé, pas encore active.
    Loaded,
    /// La scène qui vit — ses entités sont dans le World.
    Active,
    /// En cours de déchargement.
    Unloading,
    /// Invalide ou en échec — inutilisable en l'état.
    Failed,
}

/// Une scène : elle DÉCRIT et ORGANISE une portion de monde — elle ne
/// possède jamais les données vivantes. Les entités, composants,
/// ressources et messages appartiennent au `World` (unique, tenu par le
/// moteur) ; la scène possède son identité (`SceneId`, stable et
/// sérialisable), ses métadonnées et son état de cycle de vie. Détruire
/// une scène ne peut pas corrompre le monde — jamais un second ECS.
#[derive(Debug)]
pub struct Scene {
    id: SceneId,
    name: String,
    state: SceneState,
}

impl Scene {
    /// Crée une scène vide, identifiée par son nom logique (convention :
    /// minuscules, séparateur `/` — ex. `maps/spawn`).
    pub fn new(name: &str) -> Self {
        Self {
            id: SceneId::from_name(name),
            name: name.to_owned(),
            state: SceneState::Empty,
        }
    }

    /// L'identité stable de la scène — la valeur qui la distingue,
    /// indépendante de tout chemin disque.
    pub fn id(&self) -> SceneId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn state(&self) -> SceneState {
        self.state
    }

    /// La POLITIQUE du cycle de vie appartient au `SceneManager` (même
    /// crate) — l'extérieur ne peut pas corrompre un état ; les
    /// transitions intrinsèques (unload) restent ici.
    pub(crate) fn set_state(&mut self, state: SceneState) {
        self.state = state;
    }

    /// Spawne une entité appartenant à cette scène — le spawn du World et
    /// le composant `SceneMember` en un geste. L'état ne change pas :
    /// `Empty` signifie « rien de chargé » (charger appartient à la
    /// persistance), et le contenu se requête en vif via [`Scene::members`].
    pub fn spawn(&self, world: &mut World) -> ChaosResult<Entity> {
        self.ensure_usable()?;
        let entity = world.spawn()?;
        world.insert(entity, SceneMember::new(self.id))?;
        Ok(entity)
    }

    /// Revendique une entité existante : une globale devient membre, un
    /// membre d'une autre scène est re-domicilié — l'ancienne scène est
    /// rendue, jamais un basculement silencieux. Une entité morte, périmée
    /// ou forgée est une erreur explicite (la garantie d'écriture du
    /// World, avec le contexte de la scène).
    pub fn adopt(&self, world: &mut World, entity: Entity) -> ChaosResult<Option<SceneId>> {
        self.ensure_usable()?;
        match world.insert(entity, SceneMember::new(self.id)) {
            Ok(previous) => Ok(previous.map(|member| member.scene())),
            Err(error) => Err(ChaosError::Scene(format!(
                "cannot adopt {entity} into scene '{}': {error}",
                self.name
            ))),
        }
    }

    /// Les entités appartenant à cette scène — la requête filtrée, jamais
    /// une liste parallèle : une entité despawnée disparaît d'ici par
    /// construction.
    pub fn members<'world>(&self, world: &'world World) -> impl Iterator<Item = Entity> + 'world {
        let id = self.id;
        world
            .query::<SceneMember>()
            .filter(move |(_, member)| member.scene() == id)
            .map(|(entity, _)| entity)
    }

    /// Libère un membre de cette scène SANS le despawner : il devient une
    /// entité globale/persistante — il survivra aux déchargements. C'est
    /// L'OUTIL de la préservation entre changements de scène (le transfert,
    /// lui, c'est [`Scene::adopt`]). Libérer une entité qui n'appartient
    /// pas à CETTE scène (globale, ou membre d'une autre) est une erreur
    /// explicite — pas de vol inter-scènes.
    pub fn release(&self, world: &mut World, entity: Entity) -> ChaosResult<()> {
        if !self.contains(world, entity) {
            return Err(ChaosError::Scene(format!(
                "cannot release {entity} from scene '{}': it is not one of its members",
                self.name
            )));
        }
        world.remove::<SceneMember>(entity);
        Ok(())
    }

    /// L'appartenance d'une entité à CETTE scène — générationnelle par
    /// construction (une poignée périmée ne résout jamais).
    pub fn contains(&self, world: &World, entity: Entity) -> bool {
        world
            .get::<SceneMember>(entity)
            .is_some_and(|member| member.scene() == self.id)
    }

    /// Décharge la scène : despawne tous ses membres (et eux seuls) et
    /// rend leur compte. Les premières transitions réelles du modèle :
    /// `Unloading` pendant, `Empty` après — la scène est réutilisable.
    /// Décharger une scène `Failed` est LA voie de récupération. L'échec
    /// d'un despawn est impossible par construction (les membres sortent
    /// de la requête, donc vivants) — la branche est gérée honnêtement :
    /// état `Failed`, erreur nommant la scène.
    pub fn unload(&mut self, world: &mut World) -> ChaosResult<usize> {
        self.state = SceneState::Unloading;
        let members: Vec<Entity> = self.members(world).collect();
        for entity in &members {
            if let Err(error) = world.despawn(*entity) {
                self.state = SceneState::Failed;
                return Err(ChaosError::Scene(format!(
                    "unloading scene '{}' failed: {error}",
                    self.name
                )));
            }
        }
        self.state = SceneState::Empty;
        Ok(members.len())
    }

    /// Une scène `Failed` est inutilisable en l'état — l'écrire est un
    /// bug de logique, nommé. Inatteignable aujourd'hui (le seul chemin
    /// vers `Failed` est une branche impossible par construction) : gardé
    /// comme l'épuisement u32 de l'allocateur, non testé.
    fn ensure_usable(&self) -> ChaosResult<()> {
        if self.state == SceneState::Failed {
            return Err(ChaosError::Scene(format!(
                "scene '{}' has failed and is unusable",
                self.name
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chaos_ecs::World;

    use super::*;

    #[test]
    fn a_new_scene_is_empty_identified_and_named() {
        let scene = Scene::new("maps/spawn");
        assert_eq!(scene.state(), SceneState::Empty);
        assert_eq!(scene.id(), SceneId::from_name("maps/spawn"));
        assert_eq!(scene.name(), "maps/spawn");
    }

    #[test]
    fn scene_identity_is_name_derived_and_stable_across_instances() {
        let first = Scene::new("maps/spawn");
        let second = Scene::new("maps/spawn");
        let other = Scene::new("maps/city");
        assert_eq!(first.id(), second.id());
        assert_ne!(first.id(), other.id());
    }

    #[test]
    fn the_model_names_five_distinct_states() {
        let states = [
            SceneState::Empty,
            SceneState::Loaded,
            SceneState::Active,
            SceneState::Unloading,
            SceneState::Failed,
        ];
        for (i, first) in states.iter().enumerate() {
            for second in &states[i + 1..] {
                assert_ne!(first, second);
            }
        }
    }

    #[test]
    fn destroying_a_scene_never_touches_the_world() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        {
            let scene = Scene::new("maps/spawn");
            assert_eq!(scene.state(), SceneState::Empty);
        }
        assert!(world.is_alive(first));
        assert!(world.is_alive(second));
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn spawn_creates_live_members() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        let members: Vec<Entity> = (0..3).map(|_| scene.spawn(&mut world).unwrap()).collect();
        for entity in &members {
            assert!(world.is_alive(*entity));
            assert!(scene.contains(&world, *entity));
        }
        assert_eq!(scene.members(&world).count(), 3);
    }

    #[test]
    fn members_lists_only_this_scenes_entities() {
        let mut world = World::new();
        let town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let in_town = town.spawn(&mut world).unwrap();
        let in_cave_a = cave.spawn(&mut world).unwrap();
        let in_cave_b = cave.spawn(&mut world).unwrap();
        let town_members: Vec<Entity> = town.members(&world).collect();
        let cave_members: Vec<Entity> = cave.members(&world).collect();
        assert_eq!(town_members, vec![in_town]);
        assert_eq!(cave_members.len(), 2);
        assert!(cave_members.contains(&in_cave_a));
        assert!(cave_members.contains(&in_cave_b));
    }

    #[test]
    fn contains_distinguishes_member_global_and_other_scene() {
        let mut world = World::new();
        let town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let member = town.spawn(&mut world).unwrap();
        let global = world.spawn().unwrap();
        assert!(town.contains(&world, member));
        assert!(!cave.contains(&world, member));
        assert!(!town.contains(&world, global));
        assert!(!cave.contains(&world, global));
    }

    #[test]
    fn adopt_claims_a_global_entity() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        let global = world.spawn().unwrap();
        assert_eq!(scene.adopt(&mut world, global).unwrap(), None);
        assert!(scene.contains(&world, global));
    }

    #[test]
    fn adopt_rehomes_and_returns_the_previous_scene() {
        let mut world = World::new();
        let town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let entity = town.spawn(&mut world).unwrap();
        assert_eq!(cave.adopt(&mut world, entity).unwrap(), Some(town.id()));
        assert!(cave.contains(&world, entity));
        assert!(!town.contains(&world, entity));
    }

    #[test]
    fn adopting_a_dead_entity_is_an_explicit_error() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        let entity = world.spawn().unwrap();
        world.despawn(entity).unwrap();
        let error = scene.adopt(&mut world, entity).unwrap_err();
        assert!(error.to_string().contains("'maps/spawn'"));
        assert!(error.to_string().contains("dead, stale or unknown"));
    }

    #[test]
    fn unload_despawns_every_member_and_only_them() {
        let mut world = World::new();
        let mut town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let town_a = town.spawn(&mut world).unwrap();
        let town_b = town.spawn(&mut world).unwrap();
        let in_cave = cave.spawn(&mut world).unwrap();
        let global = world.spawn().unwrap();
        assert_eq!(town.unload(&mut world).unwrap(), 2);
        assert!(!world.is_alive(town_a));
        assert!(!world.is_alive(town_b));
        assert!(world.is_alive(in_cave));
        assert!(world.is_alive(global));
        assert_eq!(town.state(), SceneState::Empty);
        assert_eq!(town.members(&world).count(), 0);
        assert_eq!(cave.members(&world).count(), 1);
    }

    #[test]
    fn despawned_members_never_linger() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        let kept_a = scene.spawn(&mut world).unwrap();
        let removed = scene.spawn(&mut world).unwrap();
        let kept_b = scene.spawn(&mut world).unwrap();
        world.despawn(removed).unwrap();
        let members: Vec<Entity> = scene.members(&world).collect();
        assert_eq!(members.len(), 2);
        assert!(members.contains(&kept_a));
        assert!(members.contains(&kept_b));
        assert_eq!(scene.unload(&mut world).unwrap(), 2);
    }

    #[test]
    fn an_unloaded_scene_is_empty_and_reusable() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        scene.spawn(&mut world).unwrap();
        scene.unload(&mut world).unwrap();
        assert_eq!(scene.state(), SceneState::Empty);
        let reborn = scene.spawn(&mut world).unwrap();
        assert!(scene.contains(&world, reborn));
        assert_eq!(scene.members(&world).count(), 1);
    }

    #[test]
    fn membership_composes_with_deferred_commands() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        let direct = scene.spawn(&mut world).unwrap();
        let scene_id = scene.id();
        let mut commands = chaos_ecs::Commands::new();
        commands.push(move |world: &mut World| {
            let entity = world.spawn()?;
            world.insert(entity, SceneMember::new(scene_id))?;
            Ok(())
        });
        commands.despawn(direct);
        commands.apply(&mut world).unwrap();
        assert_eq!(scene.members(&world).count(), 1);
        assert_eq!(scene.unload(&mut world).unwrap(), 1);
    }

    #[test]
    fn release_preserves_an_entity_across_unload() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        let survivor = scene.spawn(&mut world).unwrap();
        let doomed = scene.spawn(&mut world).unwrap();
        scene.release(&mut world, survivor).unwrap();
        assert!(!scene.contains(&world, survivor));
        assert_eq!(scene.members(&world).count(), 1);
        assert_eq!(scene.unload(&mut world).unwrap(), 1);
        assert!(world.is_alive(survivor));
        assert!(!world.is_alive(doomed));
    }

    #[test]
    fn releasing_a_non_member_is_an_explicit_error() {
        let mut world = World::new();
        let town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let global = world.spawn().unwrap();
        let error = town.release(&mut world, global).unwrap_err();
        assert!(error.to_string().contains("not one of its members"));
        let member = cave.spawn(&mut world).unwrap();
        let error = town.release(&mut world, member).unwrap_err();
        assert!(error.to_string().contains("'maps/town'"));
        assert!(cave.contains(&world, member));
    }

    #[test]
    fn release_then_adopt_transfers_between_scenes() {
        let mut world = World::new();
        let mut town = Scene::new("maps/town");
        let cave = Scene::new("maps/cave");
        let traveler = town.spawn(&mut world).unwrap();
        town.release(&mut world, traveler).unwrap();
        town.unload(&mut world).unwrap();
        assert!(world.is_alive(traveler));
        assert_eq!(cave.adopt(&mut world, traveler).unwrap(), None);
        assert!(cave.contains(&world, traveler));
    }

    #[test]
    fn repeated_unload_is_harmless() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        scene.spawn(&mut world).unwrap();
        scene.spawn(&mut world).unwrap();
        assert_eq!(scene.unload(&mut world).unwrap(), 2);
        assert_eq!(scene.unload(&mut world).unwrap(), 0);
        assert_eq!(scene.unload(&mut world).unwrap(), 0);
        assert_eq!(scene.state(), SceneState::Empty);
        assert!(world.is_empty());
    }

    #[test]
    fn unloading_an_empty_scene_is_a_clean_no_op() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/spawn");
        assert_eq!(scene.unload(&mut world).unwrap(), 0);
        assert_eq!(scene.state(), SceneState::Empty);
        assert!(world.is_empty());
    }
}
