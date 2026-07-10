use std::collections::HashMap;
use std::path::PathBuf;

use chaos_core::{AssetId, ChaosError, ChaosResult};
use log::debug;

/// Type d'une ressource — les types que le moteur sait consommer
/// aujourd'hui ; audio, etc. s'ajouteront avec leurs sous-systèmes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Texture,
    Mesh,
    Material,
    Shader,
    /// Un fichier de scène — le pipeline transporte ses octets (arrivé
    /// avec le Scene System, phase 5) ; le FORMAT appartient à
    /// chaos_scene, jamais l'inverse.
    Scene,
}

/// État d'une ressource dans son cycle de vie — le loader pilotera les
/// transitions ; `Failed` porte sa raison (debug, futur éditeur).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetState {
    Unloaded,
    Loaded,
    Failed(String),
}

/// Provenance d'une ressource — déclarative, jamais lue par le registre :
/// un fichier réel (que l'importeur lira) ou une génération par le code
/// (damiers de démo, textures builtin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetSource {
    File(PathBuf),
    Procedural,
}

/// Fiche d'une ressource connue du moteur. Lecture seule hors du registre :
/// toute mutation (transitions d'état) passe par les méthodes du registre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    name: String,
    kind: AssetKind,
    source: AssetSource,
    state: AssetState,
    version: u64,
}

impl AssetEntry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    pub fn source(&self) -> &AssetSource {
        &self.source
    }

    pub fn state(&self) -> &AssetState {
        &self.state
    }

    /// Version du contenu : 0 = déclaré jamais matérialisé, puis +1 à
    /// chaque (re)matérialisation sous la même identité. La règle des
    /// consommateurs de données dérivées : re-dériver si la version a
    /// changé ET que l'état est `Loaded` — l'invalidation pull du hot
    /// reload.
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Le registre central des ressources — la référence du moteur : quelles
/// ressources existent, où elles se trouvent, leur type, leur état.
/// Il catalogue, il ne charge rien : l'I/O appartient au loader.
#[derive(Debug, Default)]
pub struct AssetRegistry {
    entries: HashMap<AssetId, AssetEntry>,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre une ressource sous son nom logique et rend son identité
    /// (`AssetId::from_name`). Un id déjà pris est une erreur explicite qui
    /// nomme l'entrée existante — le même chemin couvre le doublon et la
    /// collision de hachage théorique : détectée, jamais silencieuse.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        kind: AssetKind,
        source: AssetSource,
    ) -> ChaosResult<AssetId> {
        let name = name.into();
        let id = AssetId::from_name(&name);
        if let Some(existing) = self.entries.get(&id) {
            return Err(ChaosError::Asset(format!(
                "cannot register '{name}': {id} is already registered by '{}'",
                existing.name()
            )));
        }
        debug!("asset '{name}' registered ({kind:?}, {id})");
        self.entries.insert(
            id,
            AssetEntry {
                name,
                kind,
                source,
                state: AssetState::Unloaded,
                version: 0,
            },
        );
        Ok(id)
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.entries.get(&id)
    }

    pub fn contains(&self, id: AssetId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Mapping inverse vivant : le nom logique d'une ressource enregistrée
    /// vers son identité.
    pub fn lookup(&self, name: &str) -> Option<AssetId> {
        let id = AssetId::from_name(name);
        self.entries.contains_key(&id).then_some(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (AssetId, &AssetEntry)> {
        self.entries.iter().map(|(id, entry)| (*id, entry))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Le nombre de ressources à l'état `Loaded` — la jauge des metrics
    /// de santé (itération V1 : pourra devenir un compteur entretenu sans
    /// changer l'API).
    pub fn loaded_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry.state(), AssetState::Loaded))
            .count()
    }

    /// Transition d'état pilotée par le loader : la ressource est
    /// matérialisée — la version de contenu s'incrémente (les données
    /// dérivées des versions précédentes sont périmées). Id inconnu →
    /// erreur explicite.
    pub fn mark_loaded(&mut self, id: AssetId) -> ChaosResult<()> {
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.state = AssetState::Loaded;
                entry.version += 1;
                Ok(())
            }
            None => Err(ChaosError::Asset(format!(
                "cannot update state of unknown asset {id}"
            ))),
        }
    }

    /// Transition d'état pilotée par le loader : le chargement a échoué,
    /// la raison est conservée. Id inconnu → erreur explicite.
    pub fn mark_failed(&mut self, id: AssetId, reason: impl Into<String>) -> ChaosResult<()> {
        self.set_state(id, AssetState::Failed(reason.into()))
    }

    /// Transition d'état pilotée par le Manager : la ressource est
    /// déchargée (ou son échec est réarmé). Id inconnu → erreur explicite.
    pub fn mark_unloaded(&mut self, id: AssetId) -> ChaosResult<()> {
        self.set_state(id, AssetState::Unloaded)
    }

    fn set_state(&mut self, id: AssetId, state: AssetState) -> ChaosResult<()> {
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.state = state;
                Ok(())
            }
            None => Err(ChaosError::Asset(format!(
                "cannot update state of unknown asset {id}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_source(path: &str) -> AssetSource {
        AssetSource::File(PathBuf::from(path))
    }

    #[test]
    fn register_then_get_exposes_the_record() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                file_source("assets/textures/brick.png"),
            )
            .unwrap();
        let entry = registry.get(id).unwrap();
        assert_eq!(entry.name(), "textures/brick");
        assert_eq!(entry.kind(), AssetKind::Texture);
        assert_eq!(
            entry.source(),
            &AssetSource::File(PathBuf::from("assets/textures/brick.png"))
        );
        assert_eq!(entry.state(), &AssetState::Unloaded);
    }

    #[test]
    fn loaded_count_follows_the_state_transitions() {
        let mut registry = AssetRegistry::new();
        let brick = registry
            .register("textures/brick", AssetKind::Texture, file_source("a.png"))
            .unwrap();
        let crate_mesh = registry
            .register("models/crate", AssetKind::Mesh, AssetSource::Procedural)
            .unwrap();
        assert_eq!(registry.loaded_count(), 0);
        registry.mark_loaded(brick).unwrap();
        assert_eq!(registry.loaded_count(), 1);
        registry.mark_loaded(crate_mesh).unwrap();
        assert_eq!(registry.loaded_count(), 2);
        registry.mark_unloaded(brick).unwrap();
        assert_eq!(registry.loaded_count(), 1);
    }

    #[test]
    fn register_derives_the_documented_id() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register("models/crate", AssetKind::Mesh, AssetSource::Procedural)
            .unwrap();
        assert_eq!(id, AssetId::from_name("models/crate"));
    }

    #[test]
    fn duplicate_name_is_rejected_naming_the_existing_entry() {
        let mut registry = AssetRegistry::new();
        registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        let error = registry
            .register(
                "textures/brick",
                AssetKind::Material,
                AssetSource::Procedural,
            )
            .unwrap_err();
        assert!(error.to_string().contains("textures/brick"));
        assert!(error.to_string().contains("already registered"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn lookup_finds_registered_names_only() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register("shaders/toon", AssetKind::Shader, AssetSource::Procedural)
            .unwrap();
        assert_eq!(registry.lookup("shaders/toon"), Some(id));
        assert_eq!(registry.lookup("shaders/unknown"), None);
    }

    #[test]
    fn iteration_lists_every_asset() {
        let mut registry = AssetRegistry::new();
        let names = ["a", "b", "c"];
        for name in names {
            registry
                .register(name, AssetKind::Texture, AssetSource::Procedural)
                .unwrap();
        }
        let listed: Vec<&str> = registry.iter().map(|(_, entry)| entry.name()).collect();
        assert_eq!(listed.len(), 3);
        for name in names {
            assert!(listed.contains(&name));
        }
    }

    #[test]
    fn mark_loaded_updates_the_state() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        registry.mark_loaded(id).unwrap();
        assert_eq!(registry.get(id).unwrap().state(), &AssetState::Loaded);
    }

    #[test]
    fn mark_failed_stores_the_reason() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        registry.mark_failed(id, "file not found").unwrap();
        assert_eq!(
            registry.get(id).unwrap().state(),
            &AssetState::Failed(String::from("file not found"))
        );
    }

    #[test]
    fn mark_unloaded_resets_the_state() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        registry.mark_loaded(id).unwrap();
        registry.mark_unloaded(id).unwrap();
        assert_eq!(registry.get(id).unwrap().state(), &AssetState::Unloaded);
    }

    #[test]
    fn mark_loaded_increments_the_version() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        assert_eq!(registry.get(id).unwrap().version(), 0);
        registry.mark_loaded(id).unwrap();
        assert_eq!(registry.get(id).unwrap().version(), 1);
        registry.mark_unloaded(id).unwrap();
        registry.mark_loaded(id).unwrap();
        assert_eq!(registry.get(id).unwrap().version(), 2);
    }

    #[test]
    fn version_is_untouched_by_failures() {
        let mut registry = AssetRegistry::new();
        let id = registry
            .register(
                "textures/brick",
                AssetKind::Texture,
                AssetSource::Procedural,
            )
            .unwrap();
        registry.mark_failed(id, "boom").unwrap();
        registry.mark_unloaded(id).unwrap();
        assert_eq!(registry.get(id).unwrap().version(), 0);
    }

    #[test]
    fn state_transitions_reject_unknown_ids() {
        let mut registry = AssetRegistry::new();
        let unknown = AssetId::from_name("ghost");
        let error = registry.mark_loaded(unknown).unwrap_err();
        assert!(error.to_string().contains("unknown asset"));
        let error = registry.mark_failed(unknown, "whatever").unwrap_err();
        assert!(error.to_string().contains("unknown asset"));
    }

    #[test]
    fn empty_registry_reports_empty() {
        let registry = AssetRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.contains(AssetId::from_name("anything")));
    }
}
