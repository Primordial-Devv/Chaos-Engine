use std::collections::HashMap;
use std::fmt;

use chaos_core::{ChaosError, ChaosResult, SceneId};
use chaos_ecs::World;

use crate::scene::{Scene, SceneState};

/// Le point d'entrée UNIQUE de la gestion des scènes : le manager POSSÈDE
/// les scènes (registre par `SceneId`) et détient la politique des
/// transitions. Plusieurs scènes peuvent être ACTIVES simultanément — les
/// couches de monde (monde + interface + intérieur) ; parmi elles, **la
/// PRINCIPALE est la plus ancienne active** (`actives[0]`) — zéro état
/// supplémentaire, déterministe. Le streaming reste explicitement différé.
///
/// L'intégrité de l'état par construction : seul `&Scene` est exposé —
/// spawn/adopt/members/contains composent librement, mais `unload` (qui
/// exige `&mut Scene`) est réservé au manager : personne ne décharge une
/// active dans son dos. Les lectures sont en `Option`, les opérations en
/// erreurs explicites nommant la scène et son état.
#[derive(Default)]
pub struct SceneManager {
    scenes: HashMap<SceneId, Scene>,
    actives: Vec<SceneId>,
}

impl SceneManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Crée ET enregistre une scène vide — le geste courant.
    pub fn create(&mut self, name: &str) -> ChaosResult<SceneId> {
        self.register(Scene::new(name))
    }

    /// Enregistre une scène existante. Un doublon d'identité est refusé en
    /// nommant l'existante.
    pub fn register(&mut self, scene: Scene) -> ChaosResult<SceneId> {
        let id = scene.id();
        if let Some(existing) = self.scenes.get(&id) {
            return Err(ChaosError::Scene(format!(
                "a scene named '{}' is already registered as {id}",
                existing.name()
            )));
        }
        self.scenes.insert(id, scene);
        Ok(id)
    }

    /// Charge une scène : la REMPLIT depuis une source de contenu —
    /// aujourd'hui une closure (le code de l'app), demain la persistance
    /// en fournira une qui lit un fichier. Exige `Empty` (recharger =
    /// décharger d'abord — l'ordre des opérations). Un populate en échec
    /// marque la scène `Failed` (contenu partiel conservé — `unload` est
    /// la récupération) et propage l'erreur avec contexte.
    pub fn load(
        &mut self,
        world: &mut World,
        id: SceneId,
        populate: impl FnOnce(&Scene, &mut World) -> ChaosResult<()>,
    ) -> ChaosResult<()> {
        let scene = self.scene_entry(id)?;
        if scene.state() != SceneState::Empty {
            return Err(ChaosError::Scene(format!(
                "cannot load scene '{}': state is {:?}, unload it first",
                scene.name(),
                scene.state()
            )));
        }
        if let Err(error) = populate(scene, world) {
            let name = scene.name().to_owned();
            self.mark(id, SceneState::Failed);
            return Err(ChaosError::Scene(format!(
                "loading scene '{name}' failed: {error}"
            )));
        }
        self.mark(id, SceneState::Loaded);
        Ok(())
    }

    /// Active une scène chargée comme couche additionnelle (la première
    /// activée est la principale). Exige `Loaded` ; une scène déjà active
    /// est une erreur explicite.
    pub fn activate(&mut self, id: SceneId) -> ChaosResult<()> {
        let scene = self.scene_entry(id)?;
        if self.actives.contains(&id) {
            return Err(ChaosError::Scene(format!(
                "scene '{}' is already active",
                scene.name()
            )));
        }
        if scene.state() != SceneState::Loaded {
            return Err(ChaosError::Scene(format!(
                "cannot activate scene '{}': state is {:?}, it must be loaded",
                scene.name(),
                scene.state()
            )));
        }
        self.mark(id, SceneState::Active);
        self.actives.push(id);
        Ok(())
    }

    /// Désactive une scène active (elle redevient `Loaded`). Non-active =
    /// erreur explicite. Désactiver la principale PROMEUT la suivante
    /// (l'ordre d'activation — déterministe).
    pub fn deactivate(&mut self, id: SceneId) -> ChaosResult<()> {
        let Some(position) = self.actives.iter().position(|active| *active == id) else {
            let name = self
                .scenes
                .get(&id)
                .map(|scene| scene.name().to_owned())
                .unwrap_or_else(|| id.to_string());
            return Err(ChaosError::Scene(format!(
                "cannot deactivate '{name}': it is not active"
            )));
        };
        self.actives.remove(position);
        self.mark(id, SceneState::Loaded);
        Ok(())
    }

    /// Décharge une scène (despawne ses membres, rend le compte). REFUSÉ
    /// sur TOUTE active — couches comprises : `deactivate` ou `replace`
    /// d'abord. `Empty` = no-op ; `Failed` = la récupération.
    pub fn unload(&mut self, world: &mut World, id: SceneId) -> ChaosResult<usize> {
        if self.actives.contains(&id) {
            let scene = self.scene_entry(id)?;
            return Err(ChaosError::Scene(format!(
                "cannot unload the active scene '{}': deactivate or replace it first",
                scene.name()
            )));
        }
        let Some(scene) = self.scenes.get_mut(&id) else {
            return Err(unknown_scene(id));
        };
        scene.unload(world)
    }

    /// Remplace LA PRINCIPALE (les couches additionnelles ne bougent pas) :
    /// la cible (exigée `Loaded`) devient la nouvelle principale, l'ancienne
    /// est désactivée PUIS DÉCHARGÉE — son état est détruit, elle reste
    /// enregistrée (`Empty`, rechargeable). Rend l'identité remplacée —
    /// `None` si rien n'était actif (activation en tête).
    pub fn replace(&mut self, world: &mut World, id: SceneId) -> ChaosResult<Option<SceneId>> {
        let target = self.scene_entry(id)?;
        if target.state() != SceneState::Loaded {
            return Err(ChaosError::Scene(format!(
                "cannot replace with scene '{}': state is {:?}, it must be loaded",
                target.name(),
                target.state()
            )));
        }
        let previous = match self.main() {
            Some(departing) => {
                self.deactivate(departing)?;
                self.unload(world, departing)?;
                Some(departing)
            }
            None => None,
        };
        self.mark(id, SceneState::Active);
        self.actives.insert(0, id);
        Ok(previous)
    }

    /// LA scène principale — la plus ancienne active. Aucune est un état
    /// légitime en lecture.
    pub fn main(&self) -> Option<SceneId> {
        self.actives.first().copied()
    }

    /// Les scènes actives, dans l'ordre d'activation (la principale en
    /// tête).
    pub fn actives(&self) -> &[SceneId] {
        &self.actives
    }

    pub fn is_active(&self, id: SceneId) -> bool {
        self.actives.contains(&id)
    }

    /// L'état courant d'une scène — inconnue → `None`.
    pub fn state_of(&self, id: SceneId) -> Option<SceneState> {
        self.scenes.get(&id).map(Scene::state)
    }

    /// La scène en partagé : spawn/adopt/members/contains composent avec
    /// (`&self` leur suffit) ; `unload` reste au manager.
    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.scenes.get(&id)
    }

    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// Le nettoyage du shutdown : décharge TOUTES les scènes en ordre trié
    /// par identité (le registre est une HashMap — le tri rend le
    /// nettoyage déterministe), vide le registre, efface l'active.
    pub fn shutdown(&mut self, world: &mut World) -> ChaosResult<()> {
        self.actives.clear();
        let mut ids: Vec<SceneId> = self.scenes.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if let Some(scene) = self.scenes.get_mut(&id) {
                scene.unload(world)?;
            }
        }
        self.scenes.clear();
        Ok(())
    }

    fn scene_entry(&self, id: SceneId) -> ChaosResult<&Scene> {
        self.scenes.get(&id).ok_or_else(|| unknown_scene(id))
    }

    fn mark(&mut self, id: SceneId, state: SceneState) {
        if let Some(scene) = self.scenes.get_mut(&id) {
            scene.set_state(state);
        }
    }
}

fn unknown_scene(id: SceneId) -> ChaosError {
    ChaosError::Scene(format!("no scene registered as {id}"))
}

impl fmt::Debug for SceneManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneManager")
            .field("scenes", &self.scenes.len())
            .field("actives", &self.actives)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::Entity;

    use super::*;

    fn loaded(manager: &mut SceneManager, world: &mut World, name: &str, count: usize) -> SceneId {
        let id = manager.create(name).unwrap();
        manager
            .load(world, id, |scene, world| {
                for _ in 0..count {
                    scene.spawn(world)?;
                }
                Ok(())
            })
            .unwrap();
        id
    }

    #[test]
    fn create_registers_an_empty_scene() {
        let mut manager = SceneManager::new();
        let id = manager.create("maps/spawn").unwrap();
        assert_eq!(id, SceneId::from_name("maps/spawn"));
        assert_eq!(manager.state_of(id), Some(SceneState::Empty));
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.main(), None);
    }

    #[test]
    fn a_duplicate_scene_is_an_explicit_error() {
        let mut manager = SceneManager::new();
        manager.create("maps/spawn").unwrap();
        let error = manager.create("maps/spawn").unwrap_err();
        assert!(error.to_string().contains("'maps/spawn'"));
        assert!(error.to_string().contains("already registered"));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn load_populates_and_marks_loaded() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 3);
        assert_eq!(manager.state_of(id), Some(SceneState::Loaded));
        assert_eq!(manager.scene(id).unwrap().members(&world).count(), 3);
    }

    #[test]
    fn loading_requires_an_empty_scene() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 1);
        let error = manager.load(&mut world, id, |_, _| Ok(())).unwrap_err();
        assert!(error.to_string().contains("Loaded"));
        assert!(error.to_string().contains("unload it first"));
    }

    #[test]
    fn loading_an_unknown_scene_is_an_explicit_error() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let error = manager
            .load(&mut world, SceneId::from_name("maps/ghost"), |_, _| Ok(()))
            .unwrap_err();
        assert!(error.to_string().contains("no scene registered"));
    }

    #[test]
    fn a_failing_load_marks_the_scene_failed() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = manager.create("maps/broken").unwrap();
        let error = manager
            .load(&mut world, id, |scene, world| {
                scene.spawn(world)?;
                Err(ChaosError::Scene(String::from("corrupted content")))
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("loading scene 'maps/broken' failed")
        );
        assert!(error.to_string().contains("corrupted content"));
        assert_eq!(manager.state_of(id), Some(SceneState::Failed));
        assert_eq!(manager.scene(id).unwrap().members(&world).count(), 1);
    }

    #[test]
    fn a_failed_scene_refuses_spawn_and_adopt() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = manager.create("maps/broken").unwrap();
        let _ = manager.load(&mut world, id, |_, _| {
            Err(ChaosError::Scene(String::from("boom")))
        });
        let scene = manager.scene(id).unwrap();
        let error = scene.spawn(&mut world).unwrap_err();
        assert!(error.to_string().contains("has failed"));
        let global = world.spawn().unwrap();
        let error = scene.adopt(&mut world, global).unwrap_err();
        assert!(error.to_string().contains("has failed"));
    }

    #[test]
    fn unload_recovers_a_failed_scene() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = manager.create("maps/broken").unwrap();
        let _ = manager.load(&mut world, id, |scene, world| {
            scene.spawn(world)?;
            Err(ChaosError::Scene(String::from("boom")))
        });
        assert_eq!(manager.unload(&mut world, id).unwrap(), 1);
        assert_eq!(manager.state_of(id), Some(SceneState::Empty));
        manager.load(&mut world, id, |_, _| Ok(())).unwrap();
        assert_eq!(manager.state_of(id), Some(SceneState::Loaded));
    }

    #[test]
    fn activate_requires_a_loaded_scene() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let empty = manager.create("maps/empty").unwrap();
        let error = manager.activate(empty).unwrap_err();
        assert!(error.to_string().contains("must be loaded"));
        let id = loaded(&mut manager, &mut world, "maps/spawn", 1);
        manager.activate(id).unwrap();
        assert_eq!(manager.main(), Some(id));
        assert_eq!(manager.state_of(id), Some(SceneState::Active));
    }

    #[test]
    fn activating_an_already_active_scene_is_an_explicit_error() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/town", 1);
        manager.activate(id).unwrap();
        let error = manager.activate(id).unwrap_err();
        assert!(error.to_string().contains("'maps/town'"));
        assert!(error.to_string().contains("already active"));
        assert_eq!(manager.actives().len(), 1);
    }

    #[test]
    fn deactivate_returns_the_scene_to_loaded() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 1);
        manager.activate(id).unwrap();
        manager.deactivate(id).unwrap();
        assert_eq!(manager.main(), None);
        assert_eq!(manager.state_of(id), Some(SceneState::Loaded));
        let error = manager.deactivate(id).unwrap_err();
        assert!(error.to_string().contains("not active"));
    }

    #[test]
    fn the_active_scene_cannot_be_unloaded_directly() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 2);
        manager.activate(id).unwrap();
        let error = manager.unload(&mut world, id).unwrap_err();
        assert!(error.to_string().contains("deactivate or replace it first"));
        manager.deactivate(id).unwrap();
        assert_eq!(manager.unload(&mut world, id).unwrap(), 2);
    }

    #[test]
    fn two_scenes_are_active_independently() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 2);
        let hud = loaded(&mut manager, &mut world, "ui/hud", 3);
        manager.activate(town).unwrap();
        manager.activate(hud).unwrap();
        assert_eq!(manager.state_of(town), Some(SceneState::Active));
        assert_eq!(manager.state_of(hud), Some(SceneState::Active));
        assert_eq!(manager.scene(town).unwrap().members(&world).count(), 2);
        assert_eq!(manager.scene(hud).unwrap().members(&world).count(), 3);
        manager.deactivate(hud).unwrap();
        assert_eq!(manager.state_of(town), Some(SceneState::Active));
        assert_eq!(manager.unload(&mut world, hud).unwrap(), 3);
        assert_eq!(manager.scene(town).unwrap().members(&world).count(), 2);
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn the_first_active_scene_is_the_main() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 1);
        let hud = loaded(&mut manager, &mut world, "ui/hud", 1);
        manager.activate(town).unwrap();
        manager.activate(hud).unwrap();
        assert_eq!(manager.main(), Some(town));
        assert_eq!(manager.actives(), &[town, hud]);
        assert!(manager.is_active(hud));
    }

    #[test]
    fn deactivating_the_main_promotes_the_next() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 1);
        let hud = loaded(&mut manager, &mut world, "ui/hud", 1);
        manager.activate(town).unwrap();
        manager.activate(hud).unwrap();
        manager.deactivate(town).unwrap();
        assert_eq!(manager.main(), Some(hud));
        assert_eq!(manager.actives(), &[hud]);
    }

    #[test]
    fn replace_swaps_only_the_main_scene() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 2);
        let hud = loaded(&mut manager, &mut world, "ui/hud", 1);
        let cave = loaded(&mut manager, &mut world, "maps/cave", 3);
        manager.activate(town).unwrap();
        manager.activate(hud).unwrap();
        let hud_member = manager.scene(hud).unwrap().members(&world).next().unwrap();
        assert_eq!(manager.replace(&mut world, cave).unwrap(), Some(town));
        assert_eq!(manager.main(), Some(cave));
        assert_eq!(manager.actives(), &[cave, hud]);
        assert_eq!(manager.state_of(town), Some(SceneState::Empty));
        assert_eq!(manager.state_of(hud), Some(SceneState::Active));
        assert!(world.is_alive(hud_member));
        assert_eq!(manager.scene(cave).unwrap().members(&world).count(), 3);
    }

    #[test]
    fn replace_switches_and_destroys_the_previous() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 2);
        let cave = loaded(&mut manager, &mut world, "maps/cave", 3);
        manager.activate(town).unwrap();
        let town_members: Vec<Entity> = manager.scene(town).unwrap().members(&world).collect();
        assert_eq!(manager.replace(&mut world, cave).unwrap(), Some(town));
        for member in town_members {
            assert!(!world.is_alive(member));
        }
        assert_eq!(manager.state_of(town), Some(SceneState::Empty));
        assert_eq!(manager.state_of(cave), Some(SceneState::Active));
        assert_eq!(manager.main(), Some(cave));
        assert_eq!(manager.scene(cave).unwrap().members(&world).count(), 3);
        assert_eq!(manager.len(), 2);
    }

    #[test]
    fn replace_with_nothing_active_just_activates() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 1);
        assert_eq!(manager.replace(&mut world, id).unwrap(), None);
        assert_eq!(manager.main(), Some(id));
    }

    #[test]
    fn replace_requires_a_loaded_target() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let empty = manager.create("maps/empty").unwrap();
        let error = manager.replace(&mut world, empty).unwrap_err();
        assert!(error.to_string().contains("must be loaded"));
    }

    #[test]
    fn replace_with_no_main_activates_at_the_head() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let id = loaded(&mut manager, &mut world, "maps/spawn", 1);
        assert_eq!(manager.replace(&mut world, id).unwrap(), None);
        assert_eq!(manager.main(), Some(id));
        assert_eq!(manager.actives(), &[id]);
    }

    #[test]
    fn no_active_scene_is_a_legitimate_state() {
        let manager = SceneManager::new();
        assert_eq!(manager.main(), None);
        assert!(manager.actives().is_empty());
        assert!(!manager.is_active(SceneId::from_name("maps/ghost")));
    }

    #[test]
    fn unload_is_refused_on_an_active_layer_too() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 1);
        let hud = loaded(&mut manager, &mut world, "ui/hud", 1);
        manager.activate(town).unwrap();
        manager.activate(hud).unwrap();
        let error = manager.unload(&mut world, hud).unwrap_err();
        assert!(error.to_string().contains("deactivate or replace it first"));
    }

    #[test]
    fn shutdown_unloads_everything() {
        let mut manager = SceneManager::new();
        let mut world = World::new();
        let town = loaded(&mut manager, &mut world, "maps/town", 2);
        loaded(&mut manager, &mut world, "maps/cave", 3);
        manager.activate(town).unwrap();
        manager.shutdown(&mut world).unwrap();
        assert!(world.is_empty());
        assert!(manager.is_empty());
        assert_eq!(manager.main(), None);
    }

    #[test]
    fn the_manager_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SceneManager>();
    }
}
