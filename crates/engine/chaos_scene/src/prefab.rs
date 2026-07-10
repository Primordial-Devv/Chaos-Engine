//! La fondation des prefabs : un sous-arbre réutilisable d'entités, de
//! composants et de relations — dans LE format de scène (`EntityData`,
//! indices de snapshot), sans identité de scène ni version. La séparation
//! asset/instances est STRUCTURELLE : le `Prefab` est de la donnée, chaque
//! instanciation spawne des entités fraîches — deux instances ne partagent
//! rien. Les overrides, variantes, imbrication avancée et synchronisation
//! asset↔instances appartiennent au futur éditeur ; le fichier prefab
//! réutilisera l'encodage texte le jour venu.

use chaos_core::{ChaosError, ChaosResult, Entity, Transform};
use chaos_ecs::World;

use crate::hierarchy;
use crate::mesh_ref::MeshRef;
use crate::scene::Scene;
use crate::serialization::{EntityData, validate_entity_records};

/// Un sous-arbre réutilisable — véhicules, personnages, bâtiments,
/// composites. La racine vit à l'indice 0 (parent `None`) et les parents
/// pointent toujours en arrière : structure auto-cohérente.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefab {
    pub name: String,
    pub entities: Vec<EntityData>,
}

impl Prefab {
    /// Capture un sous-arbre vivant en prefab — parents-avant-enfants
    /// (BFS, le patron de `despawn_recursive` : un arbre garantit zéro
    /// doublon). Les liens EXTERNES sont coupés : la racine du prefab est
    /// racine même si l'entité capturée avait un parent, et l'appartenance
    /// de scène n'est pas capturée — l'instance prendra celle de sa cible.
    pub fn capture(name: &str, world: &World, root: Entity) -> ChaosResult<Self> {
        if !world.is_alive(root) {
            return Err(ChaosError::Scene(format!(
                "cannot capture the prefab '{name}' from {root}: dead, stale or unknown"
            )));
        }
        let mut subtree = vec![root];
        let mut index = 0;
        while index < subtree.len() {
            let current = subtree[index];
            subtree.extend(hierarchy::children_of(world, current));
            index += 1;
        }
        let index_of = |entity: Entity| {
            subtree
                .iter()
                .position(|candidate| *candidate == entity)
                .map(|position| position as u32)
        };
        let entities = subtree
            .iter()
            .enumerate()
            .map(|(position, entity)| EntityData {
                transform: world.get::<Transform>(*entity).copied(),
                mesh: world.get::<MeshRef>(*entity).map(MeshRef::mesh),
                parent: if position == 0 {
                    None
                } else {
                    hierarchy::parent_of(world, *entity).and_then(index_of)
                },
            })
            .collect();
        Ok(Self {
            name: name.to_owned(),
            entities,
        })
    }

    /// Les règles partagées du format + les règles structurelles du
    /// prefab : non vide, la racine UNIQUE à l'indice 0, toute autre
    /// entité rattachée — un sous-arbre est un arbre connexe.
    pub fn validate(&self) -> ChaosResult<()> {
        let owner = format!("prefab '{}'", self.name);
        if self.entities.is_empty() {
            return Err(ChaosError::Scene(format!("{owner}: it has no entities")));
        }
        validate_entity_records(&owner, &self.entities)?;
        if self.entities[0].parent.is_some() {
            return Err(ChaosError::Scene(format!(
                "{owner}: entity #0 must be the root (it has a parent)"
            )));
        }
        for (index, entity) in self.entities.iter().enumerate().skip(1) {
            if entity.parent.is_none() {
                return Err(ChaosError::Scene(format!(
                    "{owner}: entity #{index} is a second root — a prefab is a single tree"
                )));
            }
        }
        Ok(())
    }

    /// Instancie le prefab dans une scène : valide d'abord (le monde n'est
    /// pas touché si le prefab est invalide), spawne des entités FRAÎCHES
    /// membres de la scène cible, restaure composants et hiérarchie, et
    /// rend la RACINE de l'instance — le placement est un geste de
    /// l'appelant (`world.insert(root, transform)` ; la propagation
    /// déplace toute l'instance).
    pub fn instantiate(&self, scene: &Scene, world: &mut World) -> ChaosResult<Entity> {
        self.validate()?;
        let mut spawned = Vec::with_capacity(self.entities.len());
        for _ in &self.entities {
            spawned.push(scene.spawn(world)?);
        }
        for (data, entity) in self.entities.iter().zip(&spawned) {
            if let Some(transform) = data.transform {
                world.insert(*entity, transform)?;
            }
            if let Some(mesh) = data.mesh {
                world.insert(*entity, MeshRef::new(mesh))?;
            }
            if let Some(parent) = data.parent {
                hierarchy::attach(world, *entity, spawned[parent as usize])?;
            }
        }
        Ok(spawned[0])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chaos_core::math::Vec3;
    use chaos_core::{AssetId, GlobalTransform};
    use chaos_ecs::System;

    use crate::propagation::TransformPropagation;

    use super::*;

    fn vehicle_prefab(world: &mut World) -> Prefab {
        let vehicle = world.spawn().unwrap();
        world
            .insert(
                vehicle,
                Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            )
            .unwrap();
        world
            .insert(vehicle, MeshRef::new(AssetId::from_name("demo/vehicle")))
            .unwrap();
        for side in [-1.0f32, 1.0] {
            let wheel = world.spawn().unwrap();
            world
                .insert(
                    wheel,
                    Transform::from_translation(Vec3::new(side, 0.0, 0.0)),
                )
                .unwrap();
            world
                .insert(wheel, MeshRef::new(AssetId::from_name("demo/wheel")))
                .unwrap();
            hierarchy::attach(world, wheel, vehicle).unwrap();
            let bolt = world.spawn().unwrap();
            world
                .insert(bolt, Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)))
                .unwrap();
            hierarchy::attach(world, bolt, wheel).unwrap();
        }
        Prefab::capture("prefabs/vehicle", world, vehicle).unwrap()
    }

    #[test]
    fn capture_collects_the_subtree_parents_first() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        assert_eq!(prefab.entities.len(), 5);
        assert_eq!(prefab.entities[0].parent, None);
        assert_eq!(
            prefab.entities[0].mesh,
            Some(AssetId::from_name("demo/vehicle"))
        );
        for (index, entity) in prefab.entities.iter().enumerate().skip(1) {
            let parent = entity.parent.unwrap();
            assert!((parent as usize) < index, "les parents pointent en arrière");
        }
        prefab.validate().unwrap();
    }

    #[test]
    fn capture_cuts_external_links() {
        let mut world = World::new();
        let garage = world.spawn().unwrap();
        let vehicle = world.spawn().unwrap();
        let sibling = world.spawn().unwrap();
        hierarchy::attach(&mut world, vehicle, garage).unwrap();
        hierarchy::attach(&mut world, sibling, garage).unwrap();
        let prefab = Prefab::capture("prefabs/vehicle", &world, vehicle).unwrap();
        assert_eq!(prefab.entities.len(), 1);
        assert_eq!(prefab.entities[0].parent, None);
    }

    #[test]
    fn capturing_a_dead_root_is_an_explicit_error() {
        let mut world = World::new();
        let root = world.spawn().unwrap();
        world.despawn(root).unwrap();
        let error = Prefab::capture("prefabs/ghost", &world, root).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
    }

    #[test]
    fn instantiating_twice_yields_fully_distinct_entities() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        let scene = Scene::new("maps/spawn");
        let first_root = prefab.instantiate(&scene, &mut world).unwrap();
        let second_root = prefab.instantiate(&scene, &mut world).unwrap();
        assert_ne!(first_root, second_root);

        let collect = |root: Entity, world: &World| -> HashSet<Entity> {
            let mut set = HashSet::from([root]);
            let mut queue = vec![root];
            while let Some(current) = queue.pop() {
                for child in hierarchy::children_of(world, current) {
                    set.insert(child);
                    queue.push(child);
                }
            }
            set
        };
        let first = collect(first_root, &world);
        let second = collect(second_root, &world);
        assert_eq!(first.len(), 5);
        assert_eq!(second.len(), 5);
        assert!(first.is_disjoint(&second));
        assert_eq!(hierarchy::children_of(&world, first_root).count(), 2);
        assert_eq!(hierarchy::children_of(&world, second_root).count(), 2);
    }

    #[test]
    fn instances_share_no_runtime_state() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        let scene = Scene::new("maps/spawn");
        let first = prefab.instantiate(&scene, &mut world).unwrap();
        let second = prefab.instantiate(&scene, &mut world).unwrap();
        world.get_mut::<Transform>(first).unwrap().translation = Vec3::new(50.0, 0.0, 0.0);
        assert_eq!(
            world.get::<Transform>(second).unwrap().translation,
            Vec3::new(0.0, 1.0, 0.0)
        );
    }

    #[test]
    fn instantiation_restores_components_and_hierarchy() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        let scene = Scene::new("maps/spawn");
        let root = prefab.instantiate(&scene, &mut world).unwrap();
        assert_eq!(
            world.get::<MeshRef>(root).map(MeshRef::mesh),
            Some(AssetId::from_name("demo/vehicle"))
        );
        let wheels: Vec<Entity> = hierarchy::children_of(&world, root).collect();
        assert_eq!(wheels.len(), 2);
        for wheel in wheels {
            assert_eq!(
                world.get::<MeshRef>(wheel).map(MeshRef::mesh),
                Some(AssetId::from_name("demo/wheel"))
            );
            assert_eq!(hierarchy::children_of(&world, wheel).count(), 1);
        }
    }

    #[test]
    fn instances_are_members_of_the_target_scene() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        let mut scene = Scene::new("maps/spawn");
        let first = prefab.instantiate(&scene, &mut world).unwrap();
        prefab.instantiate(&scene, &mut world).unwrap();
        assert!(scene.contains(&world, first));
        assert_eq!(scene.members(&world).count(), 10);
        assert_eq!(scene.unload(&mut world).unwrap(), 10);
        assert!(!world.is_alive(first));
    }

    #[test]
    fn validate_rejects_an_empty_prefab() {
        let prefab = Prefab {
            name: String::from("prefabs/void"),
            entities: Vec::new(),
        };
        let error = prefab.validate().unwrap_err();
        assert!(error.to_string().contains("no entities"));
    }

    #[test]
    fn validate_rejects_misplaced_or_multiple_roots() {
        let rooted = |parent: Option<u32>| EntityData {
            transform: None,
            mesh: None,
            parent,
        };
        let displaced = Prefab {
            name: String::from("prefabs/broken"),
            entities: vec![rooted(Some(1)), rooted(None)],
        };
        let error = displaced.validate().unwrap_err();
        assert!(error.to_string().contains("must be the root"));
        let twin_roots = Prefab {
            name: String::from("prefabs/broken"),
            entities: vec![rooted(None), rooted(None)],
        };
        let error = twin_roots.validate().unwrap_err();
        assert!(error.to_string().contains("second root"));
    }

    #[test]
    fn instantiate_validates_first_and_leaves_the_world_untouched() {
        let prefab = Prefab {
            name: String::from("prefabs/broken"),
            entities: Vec::new(),
        };
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        assert!(prefab.instantiate(&scene, &mut world).is_err());
        assert!(world.is_empty());
    }

    #[test]
    fn an_instance_is_placed_by_its_root() {
        let mut world = World::new();
        let prefab = vehicle_prefab(&mut world);
        let scene = Scene::new("maps/spawn");
        let root = prefab.instantiate(&scene, &mut world).unwrap();
        world
            .insert(root, Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)))
            .unwrap();
        TransformPropagation.run(&mut world).unwrap();
        let wheel = hierarchy::children_of(&world, root).next().unwrap();
        let wheel_x = world.get::<GlobalTransform>(wheel).unwrap().translation().x;
        assert!((wheel_x - 9.0).abs() < 1e-5 || (wheel_x - 11.0).abs() < 1e-5);
    }
}
