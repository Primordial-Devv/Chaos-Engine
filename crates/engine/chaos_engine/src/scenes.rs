//! La couture scènes ↔ Asset Pipeline — le moteur voit les deux, eux ne se
//! voient jamais (le patron de `chaos_engine::assets`). Le CHARGEMENT
//! passe par le pipeline (un fichier de scène est un asset
//! `AssetKind::Scene` : ses octets par `load_bytes`, l'I/O reste
//! confinée) ; le décodage et la validation par chaos_scene (pur, sans
//! I/O) ; la RÉSOLUTION des références d'assets par le registre. La
//! SAUVEGARDE écrit ici : un acte d'autoring — le pipeline reste
//! fournisseur, l'éditeur héritera de ce chemin.

use std::fs;
use std::path::Path;

use chaos_assets::AssetManager;
use chaos_core::{ChaosError, ChaosResult};
use chaos_scene::SceneData;
use log::info;

/// Sauvegarde une scène sur disque — encodage texte déterministe de
/// chaos_scene, répertoires créés au besoin, erreurs enveloppées.
pub fn save_scene(path: impl AsRef<Path>, data: &SceneData) -> ChaosResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            ChaosError::Scene(format!(
                "cannot create the scene directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    fs::write(path, data.encode()).map_err(|error| {
        ChaosError::Scene(format!(
            "cannot write the scene file {}: {error}",
            path.display()
        ))
    })?;
    info!("scene '{}' saved to {}", data.name, path.display());
    Ok(())
}

/// Charge une scène par le pipeline : les octets de l'asset déclaré,
/// l'UTF-8, le décodage et la validation de chaos_scene, puis la
/// RÉSOLUTION des références — chaque mesh référencé doit être connu du
/// registre, sinon erreur explicite nommant l'entité et l'identité.
pub fn load_scene(assets: &mut AssetManager, id: chaos_core::AssetId) -> ChaosResult<SceneData> {
    let data = {
        let bytes = assets.load_bytes(id)?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| ChaosError::Scene(format!("scene asset {id} is not valid UTF-8")))?;
        SceneData::decode(text)?
    };
    data.validate()?;
    for (index, entity) in data.entities.iter().enumerate() {
        if let Some(mesh) = entity.mesh
            && !assets.registry().contains(mesh)
        {
            return Err(ChaosError::Scene(format!(
                "scene '{}': entity #{index} references unknown asset {mesh}",
                data.name
            )));
        }
    }
    info!(
        "scene '{}' loaded ({} entities)",
        data.name,
        data.entities.len()
    );
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chaos_assets::{AssetKind, AssetSource};
    use chaos_core::{AssetId, Transform};
    use chaos_ecs::World;
    use chaos_scene::{EntityData, FORMAT_VERSION, Scene};

    use super::*;

    struct TempSceneFile {
        path: PathBuf,
    }

    impl TempSceneFile {
        fn path(test: &str) -> PathBuf {
            std::env::temp_dir().join(format!("chaos_scenes_{test}_{}.cscn", std::process::id()))
        }
    }

    impl Drop for TempSceneFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn demo_data(mesh: AssetId) -> SceneData {
        SceneData {
            version: FORMAT_VERSION,
            name: String::from("scenes/roundtrip"),
            entities: vec![
                EntityData {
                    transform: Some(Transform::from_translation(chaos_core::math::Vec3::new(
                        1.0, 2.0, 3.0,
                    ))),
                    mesh: Some(mesh),
                    parent: None,
                },
                EntityData {
                    transform: None,
                    mesh: None,
                    parent: Some(0),
                },
            ],
        }
    }

    #[test]
    fn save_then_load_roundtrips_through_the_pipeline() {
        let path = TempSceneFile::path("roundtrip");
        let guard = TempSceneFile { path: path.clone() };
        let mut assets = AssetManager::new();
        let mesh = assets
            .declare("demo/cube", AssetKind::Mesh, AssetSource::Procedural)
            .unwrap();
        let data = demo_data(mesh);
        save_scene(&path, &data).unwrap();
        let scene_asset = assets
            .declare(
                "scenes/roundtrip",
                AssetKind::Scene,
                AssetSource::File(path),
            )
            .unwrap();
        let loaded = load_scene(&mut assets, scene_asset).unwrap();
        assert_eq!(loaded, data);

        let mut world = World::new();
        let scene = Scene::new("scenes/roundtrip");
        loaded.apply(&scene, &mut world).unwrap();
        assert_eq!(scene.members(&world).count(), 2);
        drop(guard);
    }

    #[test]
    fn an_unknown_asset_reference_is_an_explicit_error() {
        let path = TempSceneFile::path("unknown_ref");
        let guard = TempSceneFile { path: path.clone() };
        let mut assets = AssetManager::new();
        let ghost = AssetId::from_name("demo/ghost");
        save_scene(&path, &demo_data(ghost)).unwrap();
        let scene_asset = assets
            .declare("scenes/broken", AssetKind::Scene, AssetSource::File(path))
            .unwrap();
        let error = load_scene(&mut assets, scene_asset).unwrap_err();
        assert!(error.to_string().contains("unknown asset"));
        assert!(error.to_string().contains(&ghost.to_string()));
        drop(guard);
    }

    #[test]
    fn a_corrupted_file_fails_cleanly_through_the_seam() {
        let path = TempSceneFile::path("corrupted");
        let guard = TempSceneFile { path: path.clone() };
        fs::write(&path, b"not a scene at all").unwrap();
        let mut assets = AssetManager::new();
        let scene_asset = assets
            .declare(
                "scenes/corrupted",
                AssetKind::Scene,
                AssetSource::File(path),
            )
            .unwrap();
        let error = load_scene(&mut assets, scene_asset).unwrap_err();
        assert!(error.to_string().contains("malformed scene file"));
        drop(guard);
    }
}
