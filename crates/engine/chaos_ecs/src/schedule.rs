use std::fmt;

use chaos_core::{ChaosError, ChaosResult};

use crate::system::{System, Systems};
use crate::world::World;

struct Stage {
    name: String,
    systems: Systems,
}

/// L'ordonnancement des systèmes : des stages nommés, déclarés dans
/// l'ordre d'exécution — l'ordre global vient d'une organisation déclarée,
/// pas de l'ordre accidentel des enregistrements. Le Schedule COMPOSE des
/// `Systems` : le registre plat reste l'unité d'exécution, le Schedule
/// n'ajoute que l'organisation.
///
/// Les stages préparent les trois futurs sans en implémenter aucun : la
/// frontière de stage est le futur point de synchronisation du
/// parallélisme ; « A avant B » s'exprime à gros grain par des stages
/// différents (le graphe fin intra-stage pourra arriver sans changer le
/// contrat) ; les noms de phases moteur sont la politique du moteur — le
/// mécanisme seul vit ici, aucune enum codée en dur. Pas de
/// journalisation : `run` est LE chemin chaud.
#[derive(Default)]
pub struct Schedule {
    stages: Vec<Stage>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Déclare un stage en fin d'ordre d'exécution. Un doublon de nom est
    /// rejeté. Pas d'insertion before/after : ce besoin (plugins tiers)
    /// appartient à la plateforme, bien plus tard — limitation assumée.
    pub fn add_stage(&mut self, name: &str) -> ChaosResult<()> {
        if self.stages.iter().any(|stage| stage.name == name) {
            return Err(ChaosError::Ecs(format!(
                "a stage named '{name}' is already declared"
            )));
        }
        self.stages.push(Stage {
            name: name.to_owned(),
            systems: Systems::new(),
        });
        Ok(())
    }

    /// Enregistre un système dans un stage déclaré. L'unicité des noms est
    /// PAR stage (déléguée à `Systems::add`) : le même nom peut vivre dans
    /// deux stages — un système de synchronisation peut légitimement
    /// tourner deux fois, et l'erreur d'exécution nomme le stage.
    pub fn add_system(&mut self, stage: &str, system: impl System + 'static) -> ChaosResult<()> {
        let Some(target) = self.stages.iter_mut().find(|entry| entry.name == stage) else {
            return Err(ChaosError::Ecs(format!(
                "cannot add system '{}': no stage named '{stage}' is declared",
                system.name()
            )));
        };
        target.systems.add(system)
    }

    /// Exécute les stages dans l'ordre de déclaration, les systèmes dans
    /// l'ordre intra-stage. La première erreur arrête tout le schedule et
    /// nomme le stage (le système fautif est déjà nommé par `Systems`).
    pub fn run(&self, world: &mut World) -> ChaosResult<()> {
        for stage in &self.stages {
            stage.systems.run(world).map_err(|error| {
                ChaosError::Ecs(format!("stage '{}' failed: {error}", stage.name))
            })?;
        }
        Ok(())
    }

    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    pub fn system_count(&self) -> usize {
        self.stages.iter().map(|stage| stage.systems.len()).sum()
    }
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(
                self.stages
                    .iter()
                    .map(|stage| (&stage.name, &stage.systems)),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::resource::Resource;

    use super::*;

    #[derive(Debug, PartialEq, Default)]
    struct Trace(Vec<&'static str>);

    impl Resource for Trace {}

    struct Push(&'static str);

    impl System for Push {
        fn name(&self) -> &str {
            self.0
        }

        fn run(&self, world: &mut World) -> ChaosResult<()> {
            world.resource_mut::<Trace>().unwrap().0.push(self.0);
            Ok(())
        }
    }

    struct Fail;

    impl System for Fail {
        fn name(&self) -> &str {
            "fail"
        }

        fn run(&self, _world: &mut World) -> ChaosResult<()> {
            Err(ChaosError::Ecs(String::from("boom")))
        }
    }

    fn traced_world() -> World {
        let mut world = World::new();
        world.insert_resource(Trace::default());
        world
    }

    #[test]
    fn a_new_schedule_is_empty() {
        let schedule = Schedule::new();
        assert_eq!(schedule.stage_count(), 0);
        assert_eq!(schedule.system_count(), 0);
    }

    #[test]
    fn stages_are_declared_in_order() {
        let mut schedule = Schedule::new();
        schedule.add_stage("update").unwrap();
        schedule.add_stage("post_update").unwrap();
        assert_eq!(schedule.stage_count(), 2);
        assert_eq!(schedule.system_count(), 0);
    }

    #[test]
    fn a_duplicate_stage_name_is_an_explicit_error() {
        let mut schedule = Schedule::new();
        schedule.add_stage("update").unwrap();
        let error = schedule.add_stage("update").unwrap_err();
        assert!(error.to_string().contains("'update'"));
        assert!(error.to_string().contains("already declared"));
        assert_eq!(schedule.stage_count(), 1);
    }

    #[test]
    fn adding_a_system_to_an_unknown_stage_is_an_explicit_error() {
        let mut schedule = Schedule::new();
        let error = schedule
            .add_system("simulation", Push("mover"))
            .unwrap_err();
        assert!(error.to_string().contains("'simulation'"));
        assert!(error.to_string().contains("'mover'"));
        assert_eq!(schedule.system_count(), 0);
    }

    #[test]
    fn the_order_comes_from_the_stages_not_from_the_calls() {
        let mut schedule = Schedule::new();
        schedule.add_stage("first").unwrap();
        schedule.add_stage("second").unwrap();
        schedule.add_system("second", Push("late")).unwrap();
        schedule.add_system("first", Push("early")).unwrap();
        let mut world = traced_world();
        schedule.run(&mut world).unwrap();
        assert_eq!(world.resource::<Trace>().unwrap().0, vec!["early", "late"]);
    }

    #[test]
    fn systems_within_a_stage_keep_registration_order() {
        let mut schedule = Schedule::new();
        schedule.add_stage("update").unwrap();
        schedule.add_system("update", Push("first")).unwrap();
        schedule.add_system("update", Push("second")).unwrap();
        let mut world = traced_world();
        schedule.run(&mut world).unwrap();
        assert_eq!(
            world.resource::<Trace>().unwrap().0,
            vec!["first", "second"]
        );
    }

    #[test]
    fn the_same_system_name_may_live_in_two_stages() {
        let mut schedule = Schedule::new();
        schedule.add_stage("pre").unwrap();
        schedule.add_stage("post").unwrap();
        schedule.add_system("pre", Push("sync")).unwrap();
        schedule.add_system("post", Push("sync")).unwrap();
        assert_eq!(schedule.system_count(), 2);
        let mut world = traced_world();
        schedule.run(&mut world).unwrap();
        assert_eq!(world.resource::<Trace>().unwrap().0, vec!["sync", "sync"]);
    }

    #[test]
    fn a_duplicate_system_name_within_a_stage_is_an_explicit_error() {
        let mut schedule = Schedule::new();
        schedule.add_stage("update").unwrap();
        schedule.add_system("update", Push("mover")).unwrap();
        let error = schedule.add_system("update", Push("mover")).unwrap_err();
        assert!(error.to_string().contains("already registered"));
        assert_eq!(schedule.system_count(), 1);
    }

    #[test]
    fn a_failure_stops_the_schedule_and_names_stage_and_system() {
        let mut schedule = Schedule::new();
        schedule.add_stage("before").unwrap();
        schedule.add_stage("failing").unwrap();
        schedule.add_stage("after").unwrap();
        schedule.add_system("before", Push("ran")).unwrap();
        schedule.add_system("failing", Fail).unwrap();
        schedule.add_system("after", Push("never")).unwrap();
        let mut world = traced_world();
        let error = schedule.run(&mut world).unwrap_err();
        assert!(error.to_string().contains("stage 'failing' failed"));
        assert!(error.to_string().contains("system 'fail' failed"));
        assert!(error.to_string().contains("boom"));
        assert_eq!(world.resource::<Trace>().unwrap().0, vec!["ran"]);
    }

    #[test]
    fn running_an_empty_schedule_is_a_no_op() {
        let schedule = Schedule::new();
        let mut world = World::new();
        schedule.run(&mut world).unwrap();
        assert!(world.is_empty());
    }

    #[test]
    fn the_schedule_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Schedule>();
    }
}
