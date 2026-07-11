//! Le format de scène : une représentation PUR DONNÉES, indépendante de
//! l'état runtime — jamais d'Entity (index+génération sont des poignées de
//! session), jamais de pointeur, de handle GPU ni de détail backend. Les
//! entités sont un `Vec`, le parent un INDICE dans ce vec ; à la
//! reconstruction, des entités fraîches sont spawnées et la carte
//! indice→Entity recâble la hiérarchie. `GlobalTransform` n'est jamais
//! sérialisé : calculé, la propagation le reconstruit. Le champ `version`
//! et la validation rendent l'évolution possible (les références d'assets
//! entreront au format avec leurs composants) sans migration anticipée.
//! L'encodage disque appartient à la persistance.

use std::collections::HashMap;

use chaos_core::{AssetId, ChaosError, ChaosResult, Entity, Transform};
use chaos_ecs::World;

use crate::hierarchy;
use crate::mesh_ref::MeshRef;
use crate::scene::Scene;

/// La version courante du format — toute donnée d'une autre version est
/// rejetée par la validation (la porte d'évolutivité : les versions
/// futures y brancheront leurs migrations).
pub const FORMAT_VERSION: u32 = 1;

/// L'instantané sérialisable d'une scène. Le NOM est l'identité source :
/// le `SceneId` se recalcule (jamais stocké — aucune désynchronisation
/// possible). Lisible par les outils : tout est public et plat.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneData {
    pub version: u32,
    pub name: String,
    pub entities: Vec<EntityData>,
}

/// Une entité du snapshot — l'appartenance est implicite (tout ce qui est
/// capturé est membre), le parent est un indice local au snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityData {
    /// `None` : membre de pur groupement.
    pub transform: Option<Transform>,
    /// La référence de mesh — une identité stable, jamais un handle GPU.
    pub mesh: Option<AssetId>,
    /// Indice dans `SceneData::entities` — jamais une Entity runtime.
    pub parent: Option<u32>,
}

impl SceneData {
    /// Capture l'état sérialisable d'une scène — infaillible (lectures
    /// seules). Les membres sont TRIÉS par entité : deux captures du même
    /// monde sont égales (déterminisme). Un parent hors du snapshot
    /// (globale ou autre scène) est capturé racine : la persistance
    /// capture la structure INTERNE de la scène.
    pub fn capture(scene: &Scene, world: &World) -> Self {
        let mut members: Vec<Entity> = scene.members(world).collect();
        members.sort_unstable();
        let index_of: HashMap<Entity, u32> = members
            .iter()
            .enumerate()
            .map(|(index, entity)| (*entity, index as u32))
            .collect();
        let entities = members
            .iter()
            .map(|entity| EntityData {
                transform: world.get::<Transform>(*entity).copied(),
                mesh: world.get::<MeshRef>(*entity).map(MeshRef::mesh),
                parent: hierarchy::parent_of(world, *entity)
                    .and_then(|parent| index_of.get(&parent).copied()),
            })
            .collect();
        Self {
            version: FORMAT_VERSION,
            name: scene.name().to_owned(),
            entities,
        }
    }

    /// Détecte les données invalides, hors de tout World (les outils
    /// valideront des fichiers) : version inconnue, parent hors bornes,
    /// auto-parenté, cycle de parents, transform non fini.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.version != FORMAT_VERSION {
            return Err(ChaosError::Scene(format!(
                "scene '{}': unsupported format version {} (supported: {FORMAT_VERSION})",
                self.name, self.version
            )));
        }
        validate_entity_records(&format!("scene '{}'", self.name), &self.entities)
    }

    /// Reconstruit le contenu dans une scène : valide D'ABORD (le monde
    /// n'est pas touché si les données sont invalides), spawne les membres
    /// (`scene.spawn` — l'appartenance en un geste), puis recâble
    /// transforms et hiérarchie via la carte indice→Entity. Volontairement
    /// additif : la politique (scène vide, états) appartient au
    /// `SceneManager` — `apply` est exactement une source `populate` :
    /// `manager.load(world, id, |scene, world| data.apply(scene, world))`.
    pub fn apply(&self, scene: &Scene, world: &mut World) -> ChaosResult<()> {
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
        Ok(())
    }

    /// Encode en texte — déterministe : ordre fixe des champs
    /// (transform, mesh, parent), entités dans l'ordre du Vec, flottants
    /// en représentation la plus courte qui reboucle bit-exact (la
    /// garantie de std), identités en 16 hexadécimaux.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("chaos-scene {}\n", self.version));
        out.push_str(&format!("name {}\n", self.name));
        for entity in &self.entities {
            out.push_str("entity\n");
            if let Some(t) = &entity.transform {
                out.push_str(&format!(
                    "transform {} {} {} {} {} {} {} {} {} {}\n",
                    t.translation.x,
                    t.translation.y,
                    t.translation.z,
                    t.rotation.x,
                    t.rotation.y,
                    t.rotation.z,
                    t.rotation.w,
                    t.scale.x,
                    t.scale.y,
                    t.scale.z
                ));
            }
            if let Some(mesh) = entity.mesh {
                out.push_str(&format!("mesh {:016x}\n", mesh.value()));
            }
            if let Some(parent) = entity.parent {
                out.push_str(&format!("parent {parent}\n"));
            }
        }
        out
    }

    /// Décode le texte — parseur STRICT, malformations nommées (le
    /// précédent des importeurs) : en-tête, ordre des champs, comptes,
    /// hexadécimal. La sémantique (bornes, cycles, non-fini, version
    /// supportée) reste à [`SceneData::validate`] — les deux tournent à la
    /// couture.
    pub fn decode(text: &str) -> ChaosResult<Self> {
        let mut lines = text.lines();
        let header = lines.next().ok_or_else(|| malformed("empty scene file"))?;
        let version = header
            .strip_prefix("chaos-scene ")
            .ok_or_else(|| malformed("the header must be 'chaos-scene <version>'"))?
            .parse::<u32>()
            .map_err(|_| malformed("the header version is not a number"))?;
        let name = lines
            .next()
            .and_then(|line| line.strip_prefix("name "))
            .ok_or_else(|| malformed("the second line must be 'name <scene name>'"))?
            .to_owned();
        let mut entities: Vec<EntityData> = Vec::new();
        for line in lines {
            if line == "entity" {
                entities.push(EntityData {
                    transform: None,
                    mesh: None,
                    parent: None,
                });
                continue;
            }
            let Some(current) = entities.last_mut() else {
                return Err(malformed(&format!("field outside of any entity: '{line}'")));
            };
            if let Some(rest) = line.strip_prefix("transform ") {
                if current.transform.is_some() || current.mesh.is_some() || current.parent.is_some()
                {
                    return Err(malformed("'transform' must come first and only once"));
                }
                current.transform = Some(parse_transform(rest)?);
            } else if let Some(rest) = line.strip_prefix("mesh ") {
                if current.mesh.is_some() || current.parent.is_some() {
                    return Err(malformed("'mesh' must come before 'parent', only once"));
                }
                let value = u64::from_str_radix(rest, 16)
                    .map_err(|_| malformed(&format!("invalid mesh asset id: '{rest}'")))?;
                current.mesh = Some(AssetId::from_raw(value));
            } else if let Some(rest) = line.strip_prefix("parent ") {
                if current.parent.is_some() {
                    return Err(malformed("'parent' appears twice"));
                }
                current.parent = Some(
                    rest.parse::<u32>()
                        .map_err(|_| malformed(&format!("invalid parent index: '{rest}'")))?,
                );
            } else {
                return Err(malformed(&format!("unknown directive: '{line}'")));
            }
        }
        Ok(Self {
            version,
            name,
            entities,
        })
    }
}

/// Les règles partagées des enregistrements d'entités — le format de
/// scène et les prefabs valident le même socle : parents en bornes,
/// auto-parenté, cycles, transforms finis.
pub(crate) fn validate_entity_records(owner: &str, entities: &[EntityData]) -> ChaosResult<()> {
    let count = entities.len();
    for (index, entity) in entities.iter().enumerate() {
        if let Some(parent) = entity.parent {
            if parent as usize >= count {
                return Err(ChaosError::Scene(format!(
                    "{owner}: entity #{index} references parent #{parent}, out of bounds ({count} entities)"
                )));
            }
            if parent as usize == index {
                return Err(ChaosError::Scene(format!(
                    "{owner}: entity #{index} cannot be its own parent"
                )));
            }
        }
        if let Some(transform) = &entity.transform
            && !(transform.translation.is_finite()
                && transform.rotation.is_finite()
                && transform.scale.is_finite())
        {
            return Err(ChaosError::Scene(format!(
                "{owner}: entity #{index} has a non-finite transform"
            )));
        }
    }
    for start in 0..count {
        let mut cursor = entities[start].parent;
        let mut steps = 0;
        while let Some(parent) = cursor {
            steps += 1;
            if steps > count {
                return Err(ChaosError::Scene(format!(
                    "{owner}: the parent chain of entity #{start} contains a cycle"
                )));
            }
            cursor = entities[parent as usize].parent;
        }
    }
    Ok(())
}

fn malformed(reason: &str) -> ChaosError {
    ChaosError::Scene(format!("malformed scene file: {reason}"))
}

fn parse_transform(fields: &str) -> ChaosResult<Transform> {
    let values: Vec<f32> = fields
        .split_whitespace()
        .map(|field| {
            field
                .parse::<f32>()
                .map_err(|_| malformed(&format!("invalid transform number: '{field}'")))
        })
        .collect::<ChaosResult<Vec<f32>>>()?;
    if values.len() != 10 {
        return Err(malformed(&format!(
            "a transform needs 10 numbers, got {}",
            values.len()
        )));
    }
    Ok(Transform {
        translation: chaos_core::math::Vec3::new(values[0], values[1], values[2]),
        rotation: chaos_core::math::Quat::from_xyzw(values[3], values[4], values[5], values[6]),
        scale: chaos_core::math::Vec3::new(values[7], values[8], values[9]),
    })
}

#[cfg(test)]
mod tests {
    use chaos_core::math::Vec3;

    use crate::manager::SceneManager;
    use crate::scene::SceneState;

    use super::*;

    fn populated(world: &mut World) -> Scene {
        let scene = Scene::new("maps/spawn");
        let root = scene.spawn(world).unwrap();
        let child = scene.spawn(world).unwrap();
        world
            .insert(root, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)))
            .unwrap();
        world
            .insert(child, Transform::from_translation(Vec3::new(0.5, 0.0, 0.0)))
            .unwrap();
        hierarchy::attach(world, child, root).unwrap();
        scene
    }

    #[test]
    fn capturing_a_scene_without_members_is_empty() {
        let world = World::new();
        let scene = Scene::new("maps/void");
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.version, FORMAT_VERSION);
        assert_eq!(data.name, "maps/void");
        assert!(data.entities.is_empty());
        data.validate().unwrap();
    }

    #[test]
    fn capture_collects_members_transforms_and_hierarchy_as_indices() {
        let mut world = World::new();
        let scene = populated(&mut world);
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.entities.len(), 2);
        let roots: Vec<&EntityData> = data
            .entities
            .iter()
            .filter(|entity| entity.parent.is_none())
            .collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].transform.unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
        let child = data
            .entities
            .iter()
            .find(|entity| entity.parent.is_some())
            .unwrap();
        assert_eq!(
            child.transform.unwrap().translation,
            Vec3::new(0.5, 0.0, 0.0)
        );
        let parent_index = child.parent.unwrap() as usize;
        assert!(data.entities[parent_index].parent.is_none());
    }

    #[test]
    fn capture_is_deterministic() {
        let mut world = World::new();
        let scene = populated(&mut world);
        assert_eq!(
            SceneData::capture(&scene, &world),
            SceneData::capture(&scene, &world)
        );
    }

    #[test]
    fn a_parent_outside_the_snapshot_is_captured_as_root() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        let member = scene.spawn(&mut world).unwrap();
        let global = world.spawn().unwrap();
        hierarchy::attach(&mut world, member, global).unwrap();
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.entities.len(), 1);
        assert_eq!(data.entities[0].parent, None);
    }

    #[test]
    fn the_roundtrip_preserves_everything_supported() {
        let mut world = World::new();
        let scene = populated(&mut world);
        world.spawn().unwrap();
        let data = SceneData::capture(&scene, &world);

        let mut fresh_world = World::new();
        let fresh_scene = Scene::new("maps/spawn");
        data.apply(&fresh_scene, &mut fresh_world).unwrap();
        let rebuilt = SceneData::capture(&fresh_scene, &fresh_world);
        assert_eq!(data, rebuilt);

        assert_eq!(fresh_scene.members(&fresh_world).count(), 2);
        let child = fresh_scene
            .members(&fresh_world)
            .find(|entity| hierarchy::parent_of(&fresh_world, *entity).is_some())
            .unwrap();
        let parent = hierarchy::parent_of(&fresh_world, child).unwrap();
        assert!(fresh_scene.contains(&fresh_world, parent));
        assert_eq!(
            fresh_world.get::<Transform>(child).unwrap().translation,
            Vec3::new(0.5, 0.0, 0.0)
        );
        assert_eq!(
            fresh_world.get::<Transform>(parent).unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn a_member_without_transform_roundtrips() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        scene.spawn(&mut world).unwrap();
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.entities[0].transform, None);
        let mut fresh_world = World::new();
        let fresh_scene = Scene::new("maps/spawn");
        data.apply(&fresh_scene, &mut fresh_world).unwrap();
        assert_eq!(SceneData::capture(&fresh_scene, &fresh_world), data);
    }

    #[test]
    fn apply_composes_with_the_manager() {
        let mut world = World::new();
        let scene = populated(&mut world);
        let data = SceneData::capture(&scene, &world);

        let mut manager = SceneManager::new();
        let mut fresh_world = World::new();
        let id = manager.create("maps/spawn").unwrap();
        manager
            .load(&mut fresh_world, id, |scene, world| {
                data.apply(scene, world)
            })
            .unwrap();
        assert_eq!(manager.state_of(id), Some(SceneState::Loaded));
        assert_eq!(manager.scene(id).unwrap().members(&fresh_world).count(), 2);
    }

    #[test]
    fn validate_rejects_an_unsupported_version() {
        let data = SceneData {
            version: 999,
            name: String::from("maps/future"),
            entities: Vec::new(),
        };
        let error = data.validate().unwrap_err();
        assert!(error.to_string().contains("version 999"));
    }

    #[test]
    fn validate_rejects_an_out_of_bounds_parent() {
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("maps/broken"),
            entities: vec![EntityData {
                transform: None,
                mesh: None,
                parent: Some(7),
            }],
        };
        let error = data.validate().unwrap_err();
        assert!(error.to_string().contains("out of bounds"));
    }

    #[test]
    fn validate_rejects_a_self_parent() {
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("maps/broken"),
            entities: vec![EntityData {
                transform: None,
                mesh: None,
                parent: Some(0),
            }],
        };
        let error = data.validate().unwrap_err();
        assert!(error.to_string().contains("its own parent"));
    }

    #[test]
    fn validate_rejects_parent_cycles() {
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("maps/broken"),
            entities: vec![
                EntityData {
                    transform: None,
                    mesh: None,
                    parent: Some(1),
                },
                EntityData {
                    transform: None,
                    mesh: None,
                    parent: Some(0),
                },
            ],
        };
        let error = data.validate().unwrap_err();
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn apply_validates_first_and_leaves_the_world_untouched() {
        let nan = Transform::from_translation(Vec3::new(f32::NAN, 0.0, 0.0));
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("maps/broken"),
            entities: vec![EntityData {
                transform: Some(nan),
                mesh: None,
                parent: None,
            }],
        };
        let error = data.validate().unwrap_err();
        assert!(error.to_string().contains("non-finite"));
        let mut world = World::new();
        let scene = Scene::new("maps/broken");
        assert!(data.apply(&scene, &mut world).is_err());
        assert!(world.is_empty());
    }

    #[test]
    fn mesh_references_are_captured_and_restored() {
        let mut world = World::new();
        let scene = Scene::new("maps/spawn");
        let entity = scene.spawn(&mut world).unwrap();
        let mesh = AssetId::from_name("demo/cube");
        world.insert(entity, MeshRef::new(mesh)).unwrap();
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.entities[0].mesh, Some(mesh));

        let mut fresh_world = World::new();
        let fresh_scene = Scene::new("maps/spawn");
        data.apply(&fresh_scene, &mut fresh_world).unwrap();
        let member = fresh_scene.members(&fresh_world).next().unwrap();
        assert_eq!(
            fresh_world.get::<MeshRef>(member).map(MeshRef::mesh),
            Some(mesh)
        );
        assert_eq!(SceneData::capture(&fresh_scene, &fresh_world), data);
    }

    #[test]
    fn encode_then_decode_roundtrips_bit_exact() {
        use chaos_core::math::Quat;
        let data = SceneData {
            version: FORMAT_VERSION,
            name: String::from("scenes/demo"),
            entities: vec![
                EntityData {
                    transform: Some(Transform {
                        translation: Vec3::new(0.1, -2.7, 1e-6),
                        rotation: Quat::from_rotation_y(0.9),
                        scale: Vec3::new(0.3, 0.3, 0.3),
                    }),
                    mesh: Some(AssetId::from_name("demo/cube")),
                    parent: None,
                },
                EntityData {
                    transform: Some(Transform::from_translation(Vec3::new(1.1, 0.35, 0.0))),
                    mesh: None,
                    parent: Some(0),
                },
                EntityData {
                    transform: None,
                    mesh: None,
                    parent: None,
                },
            ],
        };
        let text = data.encode();
        assert_eq!(SceneData::decode(&text).unwrap(), data);
    }

    #[test]
    fn encode_is_deterministic() {
        let mut world = World::new();
        let scene = populated(&mut world);
        let data = SceneData::capture(&scene, &world);
        assert_eq!(data.encode(), data.encode());
    }

    #[test]
    fn decode_rejects_a_bad_header() {
        let error = SceneData::decode("chaos-mesh 1\nname x\n").unwrap_err();
        assert!(error.to_string().contains("chaos-scene"));
        let error = SceneData::decode("").unwrap_err();
        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn decode_rejects_a_missing_name() {
        let error = SceneData::decode("chaos-scene 1\nentity\n").unwrap_err();
        assert!(error.to_string().contains("name"));
    }

    #[test]
    fn decode_rejects_a_wrong_float_count() {
        let text = "chaos-scene 1\nname x\nentity\ntransform 1 2 3\n";
        let error = SceneData::decode(text).unwrap_err();
        assert!(error.to_string().contains("10 numbers"));
    }

    #[test]
    fn decode_rejects_an_invalid_mesh_id() {
        let text = "chaos-scene 1\nname x\nentity\nmesh not-hex\n";
        let error = SceneData::decode(text).unwrap_err();
        assert!(error.to_string().contains("invalid mesh asset id"));
    }

    #[test]
    fn an_unknown_component_directive_is_rejected() {
        let text = "chaos-scene 1\nname x\nentity\nrigidbody dynamic 1.0\n";
        let error = SceneData::decode(text).unwrap_err();
        assert!(error.to_string().contains("unknown directive"));
        assert!(error.to_string().contains("rigidbody"));
    }

    #[test]
    fn decode_rejects_fields_outside_an_entity_and_broken_order() {
        let error = SceneData::decode("chaos-scene 1\nname x\nparent 0\n").unwrap_err();
        assert!(error.to_string().contains("outside of any entity"));
        let text = "chaos-scene 1\nname x\nentity\nparent 0\ntransform 0 0 0 0 0 0 1 1 1 1\n";
        let error = SceneData::decode(text).unwrap_err();
        assert!(error.to_string().contains("'transform' must come first"));
        let error =
            SceneData::decode("chaos-scene 1\nname x\nentity\nvelocity 1 2 3\n").unwrap_err();
        assert!(error.to_string().contains("unknown directive"));
    }
}
