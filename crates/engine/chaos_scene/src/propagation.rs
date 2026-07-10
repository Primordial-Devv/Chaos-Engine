//! La propagation des transforms : relie la hiérarchie (structure pure)
//! au système de transforms. `Transform` est le LOCAL (relatif au parent ;
//! au monde pour une racine), `GlobalTransform` la matrice monde calculée.
//! Le système tourne dans `stages::POST_UPDATE` du moteur : les systèmes
//! de jeu mutent les locaux en UPDATE, la propagation calcule les globaux
//! après, les subsystems lisent frais.

use chaos_core::math::Mat4;
use chaos_core::{ChaosResult, Entity, GlobalTransform, Transform};
use chaos_ecs::{System, World};

use crate::hierarchy;

/// La matrice monde d'une entité, calculée FRAÎCHE par remontée de la
/// chaîne d'ancêtres — jamais lue depuis un `GlobalTransform` possiblement
/// périmé. Un ancêtre sans `Transform` contribue l'identité (nœud de
/// groupement) ; un parent mort termine la chaîne (l'auto-cicatrisation de
/// la hiérarchie : l'enfant se lit racine).
pub(crate) fn global_matrix_of(world: &World, entity: Entity) -> Mat4 {
    let mut matrix = local_matrix_of(world, entity);
    let mut cursor = entity;
    while let Some(parent) = hierarchy::parent_of(world, cursor) {
        matrix = local_matrix_of(world, parent) * matrix;
        cursor = parent;
    }
    matrix
}

fn local_matrix_of(world: &World, entity: Entity) -> Mat4 {
    world
        .get::<Transform>(entity)
        .map(Transform::matrix)
        .unwrap_or(Mat4::IDENTITY)
}

/// Le système de propagation — un service du moteur, enregistré dans
/// `stages::POST_UPDATE`. Deux passes :
/// 1. chaque porteur de `Transform` reçoit son `GlobalTransform` composé
///    par remontée — O(n·profondeur), déterministe, sans ordre de parcours
///    à gérer (et moins cher qu'une descente avec notre stockage : les
///    enfants sont un scan par nœud). Recalcul complet par frame — les
///    dirty flags viendront avec un besoin réel de performance ;
/// 2. les `GlobalTransform` orphelins (le `Transform` a été retiré) sont
///    balayés — jamais de global périmé.
pub struct TransformPropagation;

impl System for TransformPropagation {
    fn name(&self) -> &str {
        "scene.transform_propagation"
    }

    fn run(&self, world: &mut World) -> ChaosResult<()> {
        let placed: Vec<Entity> = world
            .query::<Transform>()
            .map(|(entity, _)| entity)
            .collect();
        for entity in &placed {
            let matrix = global_matrix_of(world, *entity);
            world.insert(*entity, GlobalTransform::from_matrix(matrix))?;
        }
        let orphaned: Vec<Entity> = world
            .query::<GlobalTransform>()
            .map(|(entity, _)| entity)
            .filter(|entity| world.get::<Transform>(*entity).is_none())
            .collect();
        for entity in orphaned {
            world.remove::<GlobalTransform>(entity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chaos_core::math::{Quat, Vec3};

    use crate::hierarchy::attach;

    use super::*;

    fn run(world: &mut World) {
        TransformPropagation.run(world).unwrap();
    }

    fn global_translation(world: &World, entity: Entity) -> Vec3 {
        world.get::<GlobalTransform>(entity).unwrap().translation()
    }

    fn nearly(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn a_root_gets_its_local_as_global() {
        let mut world = World::new();
        let root = world.spawn().unwrap();
        world
            .insert(root, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)))
            .unwrap();
        run(&mut world);
        assert_eq!(global_translation(&world, root), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn a_child_composes_with_its_parent() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        attach(&mut world, child, parent).unwrap();
        run(&mut world);
        assert_eq!(global_translation(&world, child), Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn depth_composes_through_grandparents() {
        let mut world = World::new();
        let grandparent = world.spawn().unwrap();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                grandparent,
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
            )
            .unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        attach(&mut world, parent, grandparent).unwrap();
        attach(&mut world, child, parent).unwrap();
        run(&mut world);
        assert!(nearly(
            global_translation(&world, child),
            Vec3::new(0.0, 0.0, -2.0)
        ));
    }

    #[test]
    fn an_ancestor_without_transform_contributes_identity() {
        let mut world = World::new();
        let group = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(4.0, 0.0, 0.0)))
            .unwrap();
        attach(&mut world, child, group).unwrap();
        run(&mut world);
        assert_eq!(global_translation(&world, child), Vec3::new(4.0, 0.0, 0.0));
        assert!(world.get::<GlobalTransform>(group).is_none());
    }

    #[test]
    fn a_dead_parents_child_propagates_as_a_root() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(Vec3::new(9.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        attach(&mut world, child, parent).unwrap();
        world.despawn(parent).unwrap();
        run(&mut world);
        assert_eq!(global_translation(&world, child), Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn a_removed_transform_sweeps_its_global() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        world.insert(entity, Transform::IDENTITY).unwrap();
        run(&mut world);
        assert!(world.get::<GlobalTransform>(entity).is_some());
        world.remove::<Transform>(entity);
        run(&mut world);
        assert!(world.get::<GlobalTransform>(entity).is_none());
    }

    #[test]
    fn moving_the_parent_moves_the_descendants() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world.insert(parent, Transform::IDENTITY).unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        attach(&mut world, child, parent).unwrap();
        run(&mut world);
        assert_eq!(global_translation(&world, child), Vec3::new(1.0, 0.0, 0.0));
        world.get_mut::<Transform>(parent).unwrap().translation = Vec3::new(0.0, 5.0, 0.0);
        run(&mut world);
        assert_eq!(global_translation(&world, child), Vec3::new(1.0, 5.0, 0.0));
    }
}
