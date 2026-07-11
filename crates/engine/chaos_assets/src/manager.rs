use std::collections::HashMap;
use std::fs;

use chaos_core::{AssetId, ChaosError, ChaosResult};
use log::{debug, warn};

use crate::import::{AssetImporter, ImportedAsset};
use crate::importers::{GltfImporter, PpmImporter, WgslImporter};
use crate::registry::{AssetKind, AssetRegistry, AssetSource, AssetState};

/// Le gardien de la vie des assets et l'unique point d'entrée du moteur
/// pour demander une ressource : déclarer → charger → importer → servir →
/// décharger. L'I/O de ressources est confinée ici — le reste du moteur ne
/// manipule jamais un fichier, exactement comme wgpu est confiné au backend.
///
/// `load_bytes` sert les octets bruts ; `import` les décode en ressources
/// préparées via les importeurs (builtins + enregistrés).
pub struct AssetManager {
    registry: AssetRegistry,
    cache: HashMap<AssetId, Vec<u8>>,
    importers: Vec<Box<dyn AssetImporter>>,
    imported: HashMap<AssetId, ImportedAsset>,
    retained: HashMap<AssetId, usize>,
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetManager {
    /// Manager avec les importeurs builtin (WGSL, PPM, GLB) — le patron
    /// `ShaderLibrary::with_builtins`.
    pub fn new() -> Self {
        Self {
            registry: AssetRegistry::new(),
            cache: HashMap::new(),
            importers: vec![
                Box::new(WgslImporter),
                Box::new(PpmImporter),
                Box::new(GltfImporter),
            ],
            imported: HashMap::new(),
            retained: HashMap::new(),
        }
    }

    /// Enregistre un importeur supplémentaire — le point d'extension du
    /// pipeline : nouveaux formats, kinds futurs, contenu moddé.
    pub fn register_importer(&mut self, importer: Box<dyn AssetImporter>) {
        self.importers.push(importer);
    }

    /// Déclare une ressource au registre — l'enregistrement passe aussi
    /// par le Manager, point d'entrée unique.
    pub fn declare(
        &mut self,
        name: impl Into<String>,
        kind: AssetKind,
        source: AssetSource,
    ) -> ChaosResult<AssetId> {
        self.registry.register(name, kind, source)
    }

    /// Vue en lecture du registre (consultation, listage, debug).
    pub fn registry(&self) -> &AssetRegistry {
        &self.registry
    }

    /// Les octets BRUTS actuellement en cache — la « mémoire suivie
    /// disponible » des metrics de santé ; les tailles des données
    /// décodées viendront quand chaque type saura se mesurer.
    pub fn cached_bytes(&self) -> u64 {
        self.cache.values().map(|bytes| bytes.len() as u64).sum()
    }

    /// LA FERMETURE du pipeline — l'arrêt du moteur, pas un `unload` :
    /// caches vidés (bruts + importés), rétentions oubliées (**l'arrêt
    /// prime sur la rétention**), toute entrée `Loaded` marquée
    /// `Unloaded`. Les DÉCLARATIONS restent (le registre est de la
    /// métadonnée). Idempotente par construction.
    pub fn shutdown(&mut self) {
        self.cache.clear();
        self.imported.clear();
        self.retained.clear();
        let loaded: Vec<AssetId> = self
            .registry
            .iter()
            .filter(|(_, entry)| matches!(entry.state(), AssetState::Loaded))
            .map(|(id, _)| id)
            .collect();
        for id in loaded {
            if let Err(state_error) = self.registry.mark_unloaded(id) {
                debug!("asset {id} vanished during shutdown: {state_error}");
            }
        }
    }

    /// Charge (ou sert du cache) les octets bruts d'une ressource — le
    /// seul endroit du moteur qui lit un fichier de ressource. Succès →
    /// état `Loaded` ; échec d'I/O → état `Failed(raison)` et erreur
    /// explicite ; asset procédural ou inconnu → erreur explicite.
    pub fn load_bytes(&mut self, id: AssetId) -> ChaosResult<&[u8]> {
        if !self.cache.contains_key(&id) {
            let (name, source) = match self.registry.get(id) {
                Some(entry) => (entry.name().to_owned(), entry.source().clone()),
                None => {
                    return Err(ChaosError::Asset(format!("cannot load unknown asset {id}")));
                }
            };
            let bytes = match source {
                AssetSource::File(path) => match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(io_error) => {
                        let reason = format!("failed to read '{}': {io_error}", path.display());
                        self.registry.mark_failed(id, reason.as_str())?;
                        return Err(ChaosError::Asset(format!("cannot load '{name}': {reason}")));
                    }
                },
                AssetSource::Procedural => {
                    return Err(ChaosError::Asset(format!(
                        "cannot load '{name}': procedural assets are materialized by their creators, not loaded"
                    )));
                }
            };
            debug!("asset '{name}' loaded ({} bytes, {id})", bytes.len());
            self.cache.insert(id, bytes);
            self.registry.mark_loaded(id)?;
        }
        self.cache
            .get(&id)
            .map(Vec::as_slice)
            .ok_or_else(|| ChaosError::Asset(format!("asset {id} vanished from the byte cache")))
    }

    /// Octets d'une ressource déjà chargée — accès cache pur, zéro I/O.
    pub fn bytes(&self, id: AssetId) -> Option<&[u8]> {
        self.cache.get(&id).map(Vec::as_slice)
    }

    /// Importe une ressource : charge ses octets puis les décode via
    /// l'importeur qui correspond à son kind ET à l'extension de sa source
    /// (minuscules). Ressource préparée mise en cache ; échec de décodage →
    /// état `Failed(raison)` et erreur explicite ; aucun importeur, source
    /// procédurale ou id inconnu → erreur explicite.
    pub fn import(&mut self, id: AssetId) -> ChaosResult<&ImportedAsset> {
        if !self.imported.contains_key(&id) {
            let (name, kind, extension) = match self.registry.get(id) {
                Some(entry) => {
                    let extension = match entry.source() {
                        AssetSource::File(path) => path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .map(str::to_ascii_lowercase),
                        AssetSource::Procedural => {
                            return Err(ChaosError::Asset(format!(
                                "cannot import '{}': procedural assets are materialized by their creators",
                                entry.name()
                            )));
                        }
                    };
                    (entry.name().to_owned(), entry.kind(), extension)
                }
                None => {
                    return Err(ChaosError::Asset(format!(
                        "cannot import unknown asset {id}"
                    )));
                }
            };
            let Some(extension) = extension else {
                return Err(ChaosError::Asset(format!(
                    "cannot import '{name}': its file has no extension"
                )));
            };
            let importer_index = self.importers.iter().position(|importer| {
                importer.kind() == kind && importer.extensions().contains(&extension.as_str())
            });
            let Some(importer_index) = importer_index else {
                let reason =
                    format!("no importer for {kind:?} asset '{name}' (extension '{extension}')");
                self.registry.mark_failed(id, reason.as_str())?;
                return Err(ChaosError::Asset(reason));
            };
            self.load_bytes(id)?;
            let outcome = {
                let bytes = self.cache.get(&id).map(Vec::as_slice).ok_or_else(|| {
                    ChaosError::Asset(format!("asset {id} vanished from the byte cache"))
                })?;
                self.importers[importer_index].import(&name, bytes)
            };
            match outcome.and_then(|asset| asset.validate(&name).map(|()| asset)) {
                Ok(asset) => {
                    debug!("asset '{name}' imported ({kind:?}, {id})");
                    self.imported.insert(id, asset);
                }
                Err(import_error) => {
                    self.registry.mark_failed(id, import_error.to_string())?;
                    return Err(import_error);
                }
            }
        }
        self.imported
            .get(&id)
            .ok_or_else(|| ChaosError::Asset(format!("asset {id} vanished from the import cache")))
    }

    /// Ressource préparée déjà importée — accès cache pur, zéro I/O.
    pub fn imported(&self, id: AssetId) -> Option<&ImportedAsset> {
        self.imported.get(&id)
    }

    /// Importe la ressource et déclare sa possession : la rétention monte
    /// de 1 — l'asset reste vivant tant qu'elle n'est pas rendue
    /// (`release`). Le « donne-moi l'asset et garde-le » des consommateurs ;
    /// `load_bytes`/`import` sans `acquire` ne font que réchauffer le cache.
    pub fn acquire(&mut self, id: AssetId) -> ChaosResult<&ImportedAsset> {
        self.import(id)?;
        *self.retained.entry(id).or_insert(0) += 1;
        self.imported
            .get(&id)
            .ok_or_else(|| ChaosError::Asset(format!("asset {id} vanished from the import cache")))
    }

    /// Rend une possession prise par `acquire`. Rendre un asset non retenu
    /// est une erreur explicite. Le cache reste chaud — la libération
    /// effective appartient à `evict_unused`.
    pub fn release(&mut self, id: AssetId) -> ChaosResult<()> {
        match self.retained.get_mut(&id) {
            Some(count) if *count > 1 => {
                *count -= 1;
                Ok(())
            }
            Some(_) => {
                self.retained.remove(&id);
                Ok(())
            }
            None => Err(ChaosError::Asset(format!(
                "cannot release asset {id}: not retained"
            ))),
        }
    }

    /// Nombre de possessions en cours (0 si non retenu ou inconnu) —
    /// l'observabilité de la mutualisation (debug, futur éditeur, budgets).
    pub fn retain_count(&self, id: AssetId) -> usize {
        self.retained.get(&id).copied().unwrap_or(0)
    }

    /// Recharge une ressource depuis sa source SOUS LA MÊME IDENTITÉ,
    /// rétention intacte — la primitive du hot reload (un futur watcher
    /// l'appellera ; un outil de dev peut l'appeler dès maintenant). La
    /// version de contenu s'incrémente en cas de succès. Si le rechargement
    /// échoue, LA DONNÉE PRÉCÉDENTE EST CONSERVÉE : sauvegarder un fichier
    /// cassé ne casse pas la scène en cours — l'échec reste consultable
    /// (état `Failed`).
    pub fn reload(&mut self, id: AssetId) -> ChaosResult<&ImportedAsset> {
        let previous = self.imported.remove(&id);
        self.cache.remove(&id);
        match self.import(id) {
            Ok(_) => {
                debug!("asset reloaded ({id})");
                self.imported.get(&id).ok_or_else(|| {
                    ChaosError::Asset(format!("asset {id} vanished from the import cache"))
                })
            }
            Err(reload_error) => {
                if let Some(previous) = previous {
                    self.imported.insert(id, previous);
                }
                Err(reload_error)
            }
        }
    }

    /// Évince tout asset que personne ne retient : octets et ressource
    /// préparée libérés, état → `Unloaded`. Rend le nombre d'assets
    /// évincés. C'est le crochet du futur streaming — sous pression
    /// mémoire, on libère ce qui n'est pas possédé ; un accès ultérieur
    /// rechargera depuis la source.
    pub fn evict_unused(&mut self) -> usize {
        let mut candidates: Vec<AssetId> = self
            .cache
            .keys()
            .copied()
            .chain(self.imported.keys().copied())
            .filter(|id| !self.retained.contains_key(id))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        for id in &candidates {
            self.cache.remove(id);
            self.imported.remove(id);
            if let Err(state_error) = self.registry.mark_unloaded(*id) {
                warn!("evicted asset {id} could not be marked unloaded: {state_error}");
            }
        }
        if !candidates.is_empty() {
            debug!("{} unused asset(s) evicted", candidates.len());
        }
        candidates.len()
    }

    /// Décharge une ressource : octets et ressource préparée libérés,
    /// état → `Unloaded`. Refuse un asset encore retenu — la mutualisation
    /// ne se casse pas par un consommateur ; idempotent pour une ressource
    /// déclarée mais non chargée ; décharger un `Failed` réarme un
    /// rechargement. Id inconnu → erreur explicite.
    pub fn unload(&mut self, id: AssetId) -> ChaosResult<()> {
        let retained = self.retain_count(id);
        if retained > 0 {
            let name = self
                .registry
                .get(id)
                .map(|entry| entry.name().to_owned())
                .unwrap_or_else(|| id.to_string());
            return Err(ChaosError::Asset(format!(
                "cannot unload '{name}': still retained ({retained})"
            )));
        }
        if self.cache.remove(&id).is_some() {
            debug!("asset bytes released ({id})");
        }
        if self.imported.remove(&id).is_some() {
            debug!("imported asset released ({id})");
        }
        self.registry.mark_unloaded(id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::registry::AssetState;

    use super::*;

    struct TempAsset {
        path: PathBuf,
    }

    impl TempAsset {
        fn create(test: &str, contents: &[u8]) -> Self {
            let path = std::env::temp_dir()
                .join(format!("chaos_assets_{test}_{}.bin", std::process::id()));
            fs::write(&path, contents).expect("écriture du fichier de test");
            Self { path }
        }

        fn source(&self) -> AssetSource {
            AssetSource::File(self.path.clone())
        }
    }

    impl Drop for TempAsset {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn cached_bytes_track_the_raw_cache() {
        let file = TempAsset::create("cached_bytes", b"chaos-bytes");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("data/blob", AssetKind::Texture, file.source())
            .unwrap();
        assert_eq!(manager.cached_bytes(), 0);
        manager.load_bytes(id).unwrap();
        assert_eq!(manager.cached_bytes(), 11);
    }

    #[test]
    fn declared_file_asset_loads_its_bytes() {
        let file = TempAsset::create("roundtrip", b"chaos");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/brick", AssetKind::Texture, file.source())
            .unwrap();
        assert_eq!(manager.load_bytes(id).unwrap(), b"chaos");
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Loaded
        );
    }

    #[test]
    fn loaded_bytes_are_served_from_the_cache() {
        let file = TempAsset::create("cache", b"cached");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/brick", AssetKind::Texture, file.source())
            .unwrap();
        manager.load_bytes(id).unwrap();
        fs::remove_file(&file.path).unwrap();
        assert_eq!(manager.load_bytes(id).unwrap(), b"cached");
    }

    #[test]
    fn bytes_is_cache_only() {
        let file = TempAsset::create("cache_only", b"lazy");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/brick", AssetKind::Texture, file.source())
            .unwrap();
        assert!(manager.bytes(id).is_none());
        manager.load_bytes(id).unwrap();
        assert_eq!(manager.bytes(id), Some(b"lazy".as_slice()));
    }

    #[test]
    fn procedural_assets_cannot_be_loaded() {
        let mut manager = AssetManager::new();
        let id = manager
            .declare("chaos/white", AssetKind::Texture, AssetSource::Procedural)
            .unwrap();
        let error = manager.load_bytes(id).unwrap_err();
        assert!(error.to_string().contains("procedural"));
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Unloaded
        );
    }

    #[test]
    fn loading_an_unknown_id_is_an_error() {
        let mut manager = AssetManager::new();
        let error = manager.load_bytes(AssetId::from_name("ghost")).unwrap_err();
        assert!(error.to_string().contains("unknown asset"));
    }

    #[test]
    fn missing_file_marks_the_asset_failed() {
        let mut manager = AssetManager::new();
        let id = manager
            .declare(
                "textures/ghost",
                AssetKind::Texture,
                AssetSource::File(PathBuf::from("/nonexistent/chaos/ghost.png")),
            )
            .unwrap();
        let error = manager.load_bytes(id).unwrap_err();
        assert!(error.to_string().contains("failed to read"));
        match manager.registry().get(id).unwrap().state() {
            AssetState::Failed(reason) => assert!(reason.contains("ghost.png")),
            other => panic!("état inattendu : {other:?}"),
        }
    }

    #[test]
    fn unload_releases_bytes_and_resets_state() {
        let file = TempAsset::create("unload", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/brick", AssetKind::Texture, file.source())
            .unwrap();
        manager.load_bytes(id).unwrap();
        fs::write(&file.path, b"v2").unwrap();
        assert_eq!(manager.load_bytes(id).unwrap(), b"v1");
        manager.unload(id).unwrap();
        assert!(manager.bytes(id).is_none());
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Unloaded
        );
        assert_eq!(manager.load_bytes(id).unwrap(), b"v2");
    }

    #[test]
    fn unload_is_idempotent() {
        let mut manager = AssetManager::new();
        let id = manager
            .declare("chaos/white", AssetKind::Texture, AssetSource::Procedural)
            .unwrap();
        manager.unload(id).unwrap();
        manager.unload(id).unwrap();
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Unloaded
        );
    }

    #[test]
    fn declare_forwards_to_the_registry() {
        let mut manager = AssetManager::new();
        let id = manager
            .declare("models/crate", AssetKind::Mesh, AssetSource::Procedural)
            .unwrap();
        assert_eq!(manager.registry().lookup("models/crate"), Some(id));
        assert_eq!(manager.registry().len(), 1);
    }

    struct TempNamed {
        path: PathBuf,
    }

    impl TempNamed {
        fn create(file_name: &str, contents: &[u8]) -> Self {
            let path = std::env::temp_dir()
                .join(format!("chaos_assets_{}_{file_name}", std::process::id()));
            fs::write(&path, contents).expect("écriture du fichier de test");
            Self { path }
        }

        fn source(&self) -> AssetSource {
            AssetSource::File(self.path.clone())
        }
    }

    impl Drop for TempNamed {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn wgsl_asset_imports_end_to_end() {
        let file = TempNamed::create("shader.wgsl", b"fn vs_main() {}");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/custom", AssetKind::Shader, file.source())
            .unwrap();
        let ImportedAsset::Shader(text) = manager.import(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "fn vs_main() {}");
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Loaded
        );
    }

    #[test]
    fn ppm_asset_imports_end_to_end() {
        let bytes = [b"P6\n1 1\n255\n".as_slice(), &[7, 8, 9]].concat();
        let file = TempNamed::create("pixel.ppm", &bytes);
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/pixel", AssetKind::Texture, file.source())
            .unwrap();
        let ImportedAsset::Texture(data) = manager.import(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!((data.width, data.height), (1, 1));
        assert_eq!(data.pixels, vec![7, 8, 9, 255]);
    }

    #[test]
    fn missing_importer_is_a_named_failure() {
        let file = TempNamed::create("brick.png", b"fake png");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("textures/brick", AssetKind::Texture, file.source())
            .unwrap();
        let error = manager.import(id).unwrap_err();
        assert!(error.to_string().contains("no importer"));
        assert!(error.to_string().contains("png"));
        match manager.registry().get(id).unwrap().state() {
            AssetState::Failed(reason) => assert!(reason.contains("no importer")),
            other => panic!("état inattendu : {other:?}"),
        }
    }

    #[test]
    fn procedural_assets_cannot_be_imported() {
        let mut manager = AssetManager::new();
        let id = manager
            .declare("chaos/white", AssetKind::Texture, AssetSource::Procedural)
            .unwrap();
        let error = manager.import(id).unwrap_err();
        assert!(error.to_string().contains("procedural"));
    }

    struct CheckerImporter;

    impl AssetImporter for CheckerImporter {
        fn kind(&self) -> AssetKind {
            AssetKind::Texture
        }

        fn extensions(&self) -> &[&str] {
            &["chk"]
        }

        fn import(&self, _name: &str, bytes: &[u8]) -> ChaosResult<ImportedAsset> {
            Ok(ImportedAsset::Texture(crate::import::TextureData {
                width: 1,
                height: 1,
                pixels: vec![bytes.first().copied().unwrap_or(0), 0, 0, 255],
            }))
        }
    }

    #[test]
    fn registered_importers_are_routed_by_kind_and_extension() {
        let file = TempNamed::create("board.chk", &[42]);
        let mut manager = AssetManager::new();
        manager.register_importer(Box::new(CheckerImporter));
        let id = manager
            .declare("textures/board", AssetKind::Texture, file.source())
            .unwrap();
        let ImportedAsset::Texture(data) = manager.import(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(data.pixels[0], 42);
    }

    #[test]
    fn glb_asset_imports_end_to_end() {
        let bin = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>();
        let json = r#"{"asset":{"version":"2.0"},
            "buffers":[{"byteLength":36}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let mut json_bytes = json.as_bytes().to_vec();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&0x4654_6C67_u32.to_le_bytes());
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        glb.extend_from_slice(&u32::try_from(json_bytes.len()).unwrap().to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        glb.extend_from_slice(&u32::try_from(bin.len()).unwrap().to_le_bytes());
        glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
        glb.extend_from_slice(&bin);

        let file = TempNamed::create("model.glb", &glb);
        let mut manager = AssetManager::new();
        let id = manager
            .declare("models/triangle", AssetKind::Mesh, file.source())
            .unwrap();
        let ImportedAsset::Mesh(data) = manager.import(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(data.positions.len(), 3);
        assert_eq!(data.indices, vec![0, 1, 2]);
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Loaded
        );
    }

    struct RogueMeshImporter;

    impl AssetImporter for RogueMeshImporter {
        fn kind(&self) -> AssetKind {
            AssetKind::Mesh
        }

        fn extensions(&self) -> &[&str] {
            &["rogue"]
        }

        fn import(&self, _name: &str, _bytes: &[u8]) -> ChaosResult<ImportedAsset> {
            Ok(ImportedAsset::Mesh(crate::import::MeshData {
                positions: vec![[0.0, 0.0, 0.0]],
                normals: Vec::new(),
                uvs: vec![[0.0, 0.0]],
                indices: vec![0, 1, 2],
            }))
        }
    }

    #[test]
    fn invalid_imported_mesh_is_refused_at_the_gate() {
        let file = TempNamed::create("bad.rogue", b"whatever");
        let mut manager = AssetManager::new();
        manager.register_importer(Box::new(RogueMeshImporter));
        let id = manager
            .declare("models/bad", AssetKind::Mesh, file.source())
            .unwrap();
        let error = manager.import(id).unwrap_err();
        assert!(error.to_string().contains("out-of-bounds index"));
        assert!(manager.imported(id).is_none());
        match manager.registry().get(id).unwrap().state() {
            AssetState::Failed(reason) => assert!(reason.contains("out-of-bounds")),
            other => panic!("état inattendu : {other:?}"),
        }
    }

    struct RogueTextureImporter;

    impl AssetImporter for RogueTextureImporter {
        fn kind(&self) -> AssetKind {
            AssetKind::Texture
        }

        fn extensions(&self) -> &[&str] {
            &["roguetex"]
        }

        fn import(&self, _name: &str, _bytes: &[u8]) -> ChaosResult<ImportedAsset> {
            Ok(ImportedAsset::Texture(crate::import::TextureData {
                width: 2,
                height: 2,
                pixels: vec![0; 3],
            }))
        }
    }

    #[test]
    fn invalid_imported_texture_is_refused_at_the_gate() {
        let file = TempNamed::create("bad.roguetex", b"whatever");
        let mut manager = AssetManager::new();
        manager.register_importer(Box::new(RogueTextureImporter));
        let id = manager
            .declare("textures/bad", AssetKind::Texture, file.source())
            .unwrap();
        let error = manager.import(id).unwrap_err();
        assert!(error.to_string().contains("expects 16 pixel bytes"));
        assert!(manager.imported(id).is_none());
    }

    #[test]
    fn the_manager_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AssetManager>();
    }

    #[test]
    fn shutdown_closes_everything_even_retained() {
        let file = TempNamed::create("shutdown_all.wgsl", b"fn main() {}");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/closing", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        assert_eq!(manager.registry().loaded_count(), 1);
        assert!(manager.cached_bytes() > 0);
        assert_eq!(manager.retain_count(id), 1);
        manager.shutdown();
        assert_eq!(manager.registry().loaded_count(), 0);
        assert_eq!(manager.cached_bytes(), 0);
        assert_eq!(manager.retain_count(id), 0);
        assert!(manager.imported(id).is_none());
        assert_eq!(manager.registry().len(), 1);
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Unloaded
        );
    }

    #[test]
    fn shutdown_is_idempotent() {
        let file = TempNamed::create("shutdown_twice.wgsl", b"fn main() {}");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/twice", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        manager.shutdown();
        manager.shutdown();
        assert_eq!(manager.registry().loaded_count(), 0);
        assert_eq!(manager.cached_bytes(), 0);
        assert_eq!(manager.registry().len(), 1);
    }

    #[test]
    fn acquire_imports_and_retains() {
        let file = TempNamed::create("acquire.wgsl", b"fn main() {}");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/held", AssetKind::Shader, file.source())
            .unwrap();
        let ImportedAsset::Shader(text) = manager.acquire(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "fn main() {}");
        assert_eq!(manager.retain_count(id), 1);
        assert_eq!(
            manager.registry().get(id).unwrap().state(),
            &AssetState::Loaded
        );
    }

    #[test]
    fn acquired_assets_are_shared() {
        let file = TempNamed::create("shared.wgsl", b"shared");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/shared", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        fs::remove_file(&file.path).unwrap();
        manager.acquire(id).unwrap();
        assert_eq!(manager.retain_count(id), 2);
    }

    #[test]
    fn release_decrements_until_not_retained() {
        let file = TempNamed::create("release.wgsl", b"v");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/release", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        manager.acquire(id).unwrap();
        manager.release(id).unwrap();
        assert_eq!(manager.retain_count(id), 1);
        manager.release(id).unwrap();
        assert_eq!(manager.retain_count(id), 0);
        let error = manager.release(id).unwrap_err();
        assert!(error.to_string().contains("not retained"));
    }

    #[test]
    fn release_of_unretained_asset_is_an_error() {
        let mut manager = AssetManager::new();
        let error = manager.release(AssetId::from_name("ghost")).unwrap_err();
        assert!(error.to_string().contains("not retained"));
    }

    #[test]
    fn unload_refuses_retained_assets() {
        let file = TempNamed::create("locked.wgsl", b"v");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/locked", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        let error = manager.unload(id).unwrap_err();
        assert!(error.to_string().contains("still retained (1)"));
        assert!(manager.imported(id).is_some());
        manager.release(id).unwrap();
        manager.unload(id).unwrap();
        assert!(manager.imported(id).is_none());
    }

    #[test]
    fn evict_unused_frees_only_unretained() {
        let held_file = TempNamed::create("held.wgsl", b"held");
        let idle_file = TempNamed::create("idle.wgsl", b"idle");
        let mut manager = AssetManager::new();
        let held = manager
            .declare("shaders/held", AssetKind::Shader, held_file.source())
            .unwrap();
        let idle = manager
            .declare("shaders/idle", AssetKind::Shader, idle_file.source())
            .unwrap();
        manager.acquire(held).unwrap();
        manager.import(idle).unwrap();
        assert_eq!(manager.evict_unused(), 1);
        assert!(manager.imported(held).is_some());
        assert!(manager.imported(idle).is_none());
        assert!(manager.bytes(idle).is_none());
        assert_eq!(
            manager.registry().get(idle).unwrap().state(),
            &AssetState::Unloaded
        );
    }

    #[test]
    fn evicted_assets_reload_from_disk() {
        let file = TempNamed::create("stream.wgsl", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/stream", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        manager.release(id).unwrap();
        assert_eq!(manager.evict_unused(), 1);
        fs::write(&file.path, b"v2").unwrap();
        let ImportedAsset::Shader(text) = manager.acquire(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "v2");
    }

    #[test]
    fn retain_count_reports_zero_for_unknown_assets() {
        let manager = AssetManager::new();
        assert_eq!(manager.retain_count(AssetId::from_name("ghost")), 0);
    }

    #[test]
    fn reload_rereads_the_source() {
        let file = TempNamed::create("hot.wgsl", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/hot", AssetKind::Shader, file.source())
            .unwrap();
        manager.import(id).unwrap();
        assert_eq!(manager.registry().get(id).unwrap().version(), 1);
        fs::write(&file.path, b"v2").unwrap();
        let ImportedAsset::Shader(text) = manager.reload(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "v2");
        assert_eq!(manager.registry().get(id).unwrap().version(), 2);
    }

    #[test]
    fn reload_works_while_retained() {
        let file = TempNamed::create("held_hot.wgsl", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/held_hot", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        fs::write(&file.path, b"v2").unwrap();
        let ImportedAsset::Shader(text) = manager.reload(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "v2");
        assert_eq!(manager.retain_count(id), 1);
    }

    #[test]
    fn failed_reload_keeps_the_previous_data() {
        let file = TempNamed::create("broken_hot.wgsl", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/broken_hot", AssetKind::Shader, file.source())
            .unwrap();
        manager.acquire(id).unwrap();
        fs::write(&file.path, [0xff, 0xfe]).unwrap();
        let error = manager.reload(id).unwrap_err();
        assert!(error.to_string().contains("UTF-8"));
        let Some(ImportedAsset::Shader(text)) = manager.imported(id) else {
            panic!("l'ancienne donnée doit rester servie");
        };
        assert_eq!(text, "v1");
        match manager.registry().get(id).unwrap().state() {
            AssetState::Failed(reason) => assert!(reason.contains("UTF-8")),
            other => panic!("état inattendu : {other:?}"),
        }
    }

    #[test]
    fn reload_of_an_unknown_asset_is_an_error() {
        let mut manager = AssetManager::new();
        let error = manager.reload(AssetId::from_name("ghost")).unwrap_err();
        assert!(error.to_string().contains("unknown asset"));
    }

    #[test]
    fn unload_releases_the_imported_asset() {
        let file = TempNamed::create("reload.wgsl", b"v1");
        let mut manager = AssetManager::new();
        let id = manager
            .declare("shaders/reload", AssetKind::Shader, file.source())
            .unwrap();
        manager.import(id).unwrap();
        assert!(manager.imported(id).is_some());
        manager.unload(id).unwrap();
        assert!(manager.imported(id).is_none());
        fs::write(&file.path, b"v2").unwrap();
        let ImportedAsset::Shader(text) = manager.import(id).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(text, "v2");
    }
}
