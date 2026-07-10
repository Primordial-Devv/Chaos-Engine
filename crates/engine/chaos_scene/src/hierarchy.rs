//! La hiérarchie parent/enfant : UN composant sur l'enfant, jamais de
//! liste redondante côté parent — l'état redondant se désynchronise (la
//! leçon des hiérarchies doublement liées). Le World est la source de
//! vérité unique : « un seul parent » est structurel (un composant = une
//! valeur), les enfants sont une requête filtrée, et les lectures sont
//! AUTO-CICATRISANTES — aucune poignée morte ne sort de cette API. La
//! hiérarchie est une relation du World, orthogonale à l'appartenance de
//! scène : qui doit mourir avec une scène doit en être membre. Structure
//! pure — pas de propagation de transforms, rien du renderer.

use chaos_core::{ChaosError, ChaosResult, Entity, Transform};
use chaos_ecs::{Component, World};

use crate::propagation::global_matrix_of;

/// Le lien d'un enfant vers son parent direct — l'Entity complète,
/// génération incluse : jamais résolu vers un autre. Public : les closures
/// de `Commands` (et plus tard le réseau) l'insèrent comme n'importe quel
/// composant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildOf {
    parent: Entity,
}

impl ChildOf {
    pub fn new(parent: Entity) -> Self {
        Self { parent }
    }

    pub fn parent(&self) -> Entity {
        self.parent
    }
}

impl Component for ChildOf {}

/// Attache `child` sous `parent` (reparenting compris : l'ancien parent
/// est rendu — jamais un basculement silencieux). Les relations invalides
/// sont des erreurs explicites : soi-même, parent mort (un lien mort-né
/// est indéfendable), enfant mort (la garantie d'écriture du World),
/// cycle (remontée de la chaîne d'ancêtres, O(profondeur)).
pub fn attach(world: &mut World, child: Entity, parent: Entity) -> ChaosResult<Option<Entity>> {
    if child == parent {
        return Err(ChaosError::Scene(format!(
            "cannot attach {child} to itself"
        )));
    }
    if !world.is_alive(parent) {
        return Err(ChaosError::Scene(format!(
            "cannot attach {child} to {parent}: the parent is dead, stale or unknown"
        )));
    }
    let mut cursor = parent;
    while let Some(ancestor) = parent_of(world, cursor) {
        if ancestor == child {
            return Err(ChaosError::Scene(format!(
                "cannot attach {child} to {parent}: this would create a cycle"
            )));
        }
        cursor = ancestor;
    }
    match world.insert(child, ChildOf::new(parent)) {
        Ok(previous) => Ok(previous.map(|link| link.parent())),
        Err(error) => Err(ChaosError::Scene(format!(
            "cannot attach {child} to {parent}: {error}"
        ))),
    }
}

/// Comme [`attach`], mais PRÉSERVE le transform GLOBAL de l'enfant : son
/// local est recalculé (`parent_global⁻¹ × child_global`, capturé frais
/// par remontée — jamais depuis un composant possiblement périmé) puis
/// décomposé en TRS — exact tant qu'aucun cisaillement n'apparaît
/// (échelles non uniformes composées à des rotations). `attach` nu, lui,
/// préserve le LOCAL : l'entité saute si les repères diffèrent — la
/// conservation se choisit par l'opération. Enfant sans `Transform` : se
/// comporte comme `attach` (rien à préserver).
pub fn attach_keeping_global(
    world: &mut World,
    child: Entity,
    parent: Entity,
) -> ChaosResult<Option<Entity>> {
    if world.get::<Transform>(child).is_none() {
        return attach(world, child, parent);
    }
    let child_global = global_matrix_of(world, child);
    let previous = attach(world, child, parent)?;
    let parent_global = global_matrix_of(world, parent);
    let local = parent_global.inverse() * child_global;
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    world.insert(
        child,
        Transform {
            translation,
            rotation,
            scale,
        },
    )?;
    Ok(previous)
}

/// Comme [`detach`], mais le global de l'enfant devient le local de la
/// nouvelle racine — la position monde est préservée.
pub fn detach_keeping_global(world: &mut World, child: Entity) -> Option<Entity> {
    world.get::<Transform>(child)?;
    let global = global_matrix_of(world, child);
    let former = detach(world, child)?;
    let (scale, rotation, translation) = global.to_scale_rotation_translation();
    if let Some(transform) = world.get_mut::<Transform>(child) {
        *transform = Transform {
            translation,
            rotation,
            scale,
        };
    }
    Some(former)
}

/// Détache `child` de son parent et rend l'ex-parent s'il est encore
/// vivant — le détachement est en Option, et la surface publique ne rend
/// jamais de morts. Sans parent (ou enfant périmé) → `None`.
pub fn detach(world: &mut World, child: Entity) -> Option<Entity> {
    let parent = world.remove::<ChildOf>(child)?.parent();
    world.is_alive(parent).then_some(parent)
}

/// Le parent direct — auto-cicatrisant : un lien vers un parent despawné
/// se lit `None`, l'enfant redevient une racine.
pub fn parent_of(world: &World, entity: Entity) -> Option<Entity> {
    let parent = world.get::<ChildOf>(entity)?.parent();
    world.is_alive(parent).then_some(parent)
}

/// Les enfants directs — la requête filtrée, jamais une liste redondante.
/// Parent mort → vide. Aucun ordre de fratrie garanti.
pub fn children_of(world: &World, parent: Entity) -> impl Iterator<Item = Entity> + '_ {
    let alive = world.is_alive(parent);
    world
        .query::<ChildOf>()
        .filter(move |(_, link)| alive && link.parent() == parent)
        .map(|(entity, _)| entity)
}

/// Despawne `root` et TOUT son sous-arbre — le chemin propre des objets
/// composites (véhicule → roues). Collecte d'abord (l'arbre garantit zéro
/// doublon), despawne ensuite ; rend le compte. Racine morte → erreur
/// explicite ; l'échec d'un despawn collecté est impossible par
/// construction, la branche est gérée honnêtement.
pub fn despawn_recursive(world: &mut World, root: Entity) -> ChaosResult<usize> {
    if !world.is_alive(root) {
        return Err(ChaosError::Scene(format!(
            "cannot despawn the hierarchy of {root}: dead, stale or unknown"
        )));
    }
    let mut subtree = vec![root];
    let mut index = 0;
    while index < subtree.len() {
        let current = subtree[index];
        subtree.extend(children_of(world, current));
        index += 1;
    }
    for entity in &subtree {
        if let Err(error) = world.despawn(*entity) {
            return Err(ChaosError::Scene(format!(
                "despawning the hierarchy of {root} failed: {error}"
            )));
        }
    }
    Ok(subtree.len())
}

#[cfg(test)]
mod tests {
    use chaos_core::math;
    use chaos_ecs::Commands;

    use crate::scene::Scene;

    use super::*;

    #[test]
    fn attach_builds_a_tree() {
        let mut world = World::new();
        let vehicle = world.spawn().unwrap();
        let left_wheel = world.spawn().unwrap();
        let right_wheel = world.spawn().unwrap();
        assert_eq!(attach(&mut world, left_wheel, vehicle).unwrap(), None);
        assert_eq!(attach(&mut world, right_wheel, vehicle).unwrap(), None);
        assert_eq!(parent_of(&world, left_wheel), Some(vehicle));
        assert_eq!(parent_of(&world, vehicle), None);
        let children: Vec<Entity> = children_of(&world, vehicle).collect();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&left_wheel));
        assert!(children.contains(&right_wheel));
        assert_eq!(children_of(&world, left_wheel).count(), 0);
    }

    #[test]
    fn reattaching_rehomes_and_returns_the_previous_parent() {
        let mut world = World::new();
        let first = world.spawn().unwrap();
        let second = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        attach(&mut world, child, first).unwrap();
        assert_eq!(attach(&mut world, child, second).unwrap(), Some(first));
        assert_eq!(parent_of(&world, child), Some(second));
        assert_eq!(children_of(&world, first).count(), 0);
        assert_eq!(children_of(&world, second).count(), 1);
    }

    #[test]
    fn attaching_to_itself_is_an_explicit_error() {
        let mut world = World::new();
        let entity = world.spawn().unwrap();
        let error = attach(&mut world, entity, entity).unwrap_err();
        assert!(error.to_string().contains("to itself"));
    }

    #[test]
    fn cycles_are_explicit_errors() {
        let mut world = World::new();
        let a = world.spawn().unwrap();
        let b = world.spawn().unwrap();
        let c = world.spawn().unwrap();
        attach(&mut world, b, a).unwrap();
        attach(&mut world, c, b).unwrap();
        let error = attach(&mut world, a, c).unwrap_err();
        assert!(error.to_string().contains("cycle"));
        let error = attach(&mut world, a, b).unwrap_err();
        assert!(error.to_string().contains("cycle"));
        assert_eq!(parent_of(&world, a), None);
    }

    #[test]
    fn attaching_to_a_dead_parent_is_an_explicit_error() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world.despawn(parent).unwrap();
        let error = attach(&mut world, child, parent).unwrap_err();
        assert!(error.to_string().contains("the parent is dead"));
        assert_eq!(parent_of(&world, child), None);
    }

    #[test]
    fn attaching_a_dead_child_is_an_explicit_error() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world.despawn(child).unwrap();
        let error = attach(&mut world, child, parent).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
        assert_eq!(children_of(&world, parent).count(), 0);
    }

    #[test]
    fn detach_returns_the_former_parent_then_none() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        attach(&mut world, child, parent).unwrap();
        assert_eq!(detach(&mut world, child), Some(parent));
        assert_eq!(parent_of(&world, child), None);
        assert_eq!(children_of(&world, parent).count(), 0);
        assert_eq!(detach(&mut world, child), None);
    }

    #[test]
    fn children_of_a_directly_despawned_parent_read_as_roots() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        attach(&mut world, child, parent).unwrap();
        world.despawn(parent).unwrap();
        assert_eq!(parent_of(&world, child), None);
        assert_eq!(children_of(&world, parent).count(), 0);
        assert!(world.is_alive(child));
    }

    #[test]
    fn despawn_recursive_takes_the_whole_subtree() {
        let mut world = World::new();
        let vehicle = world.spawn().unwrap();
        let bystander = world.spawn().unwrap();
        let mut bolts = Vec::new();
        for _ in 0..2 {
            let wheel = world.spawn().unwrap();
            attach(&mut world, wheel, vehicle).unwrap();
            let bolt = world.spawn().unwrap();
            attach(&mut world, bolt, wheel).unwrap();
            bolts.push(bolt);
        }
        assert_eq!(despawn_recursive(&mut world, vehicle).unwrap(), 5);
        assert!(!world.is_alive(vehicle));
        for bolt in bolts {
            assert!(!world.is_alive(bolt));
        }
        assert!(world.is_alive(bystander));
        assert_eq!(world.len(), 1);
    }

    #[test]
    fn despawn_recursive_on_a_dead_root_is_an_explicit_error() {
        let mut world = World::new();
        let root = world.spawn().unwrap();
        world.despawn(root).unwrap();
        let error = despawn_recursive(&mut world, root).unwrap_err();
        assert!(error.to_string().contains("dead, stale or unknown"));
    }

    #[test]
    fn unload_composes_with_the_hierarchy() {
        let mut world = World::new();
        let mut scene = Scene::new("maps/garage");
        let vehicle = scene.spawn(&mut world).unwrap();
        let wheel = scene.spawn(&mut world).unwrap();
        attach(&mut world, wheel, vehicle).unwrap();
        let global_tag = world.spawn().unwrap();
        attach(&mut world, global_tag, vehicle).unwrap();
        assert_eq!(scene.unload(&mut world).unwrap(), 2);
        assert!(!world.is_alive(vehicle));
        assert!(!world.is_alive(wheel));
        assert!(world.is_alive(global_tag));
        assert_eq!(parent_of(&world, global_tag), None);
        assert_eq!(children_of(&world, vehicle).count(), 0);
    }

    #[test]
    fn attach_preserves_the_local_transform() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        let local = Transform::from_translation(math::Vec3::new(1.0, 0.0, 0.0));
        world
            .insert(
                parent,
                Transform::from_translation(math::Vec3::new(5.0, 0.0, 0.0)),
            )
            .unwrap();
        world.insert(child, local).unwrap();
        attach(&mut world, child, parent).unwrap();
        assert_eq!(world.get::<Transform>(child), Some(&local));
    }

    #[test]
    fn attach_keeping_global_preserves_the_world_position() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(math::Vec3::new(1.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(
                child,
                Transform::from_translation(math::Vec3::new(3.0, 0.0, 0.0)),
            )
            .unwrap();
        attach_keeping_global(&mut world, child, parent).unwrap();
        let local = world.get::<Transform>(child).unwrap();
        assert!((local.translation - math::Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5);
        let global = crate::propagation::global_matrix_of(&world, child);
        assert!((global.w_axis.truncate() - math::Vec3::new(3.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn detach_keeping_global_promotes_the_global_to_root_local() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        world
            .insert(
                parent,
                Transform::from_translation(math::Vec3::new(1.0, 0.0, 0.0)),
            )
            .unwrap();
        world
            .insert(
                child,
                Transform::from_translation(math::Vec3::new(2.0, 0.0, 0.0)),
            )
            .unwrap();
        attach(&mut world, child, parent).unwrap();
        assert_eq!(detach_keeping_global(&mut world, child), Some(parent));
        let local = world.get::<Transform>(child).unwrap();
        assert!((local.translation - math::Vec3::new(3.0, 0.0, 0.0)).length() < 1e-5);
        assert_eq!(parent_of(&world, child), None);
    }

    #[test]
    fn attach_keeping_global_without_a_child_transform_behaves_like_attach() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        assert_eq!(
            attach_keeping_global(&mut world, child, parent).unwrap(),
            None
        );
        assert_eq!(parent_of(&world, child), Some(parent));
        assert!(world.get::<Transform>(child).is_none());
    }

    #[test]
    fn hierarchy_composes_with_deferred_commands() {
        let mut world = World::new();
        let parent = world.spawn().unwrap();
        let child = world.spawn().unwrap();
        let mut commands = Commands::new();
        commands.push(move |world: &mut World| attach(world, child, parent).map(|_| ()));
        assert_eq!(parent_of(&world, child), None);
        commands.apply(&mut world).unwrap();
        assert_eq!(parent_of(&world, child), Some(parent));
    }
}
