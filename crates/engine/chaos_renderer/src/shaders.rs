use std::collections::HashMap;

use log::warn;

use crate::resources::ShaderSource;

/// Noms des shaders intégrés du moteur (namespace `chaos.`).
pub mod builtin {
    pub const VERTEX_COLOR: &str = "chaos.vertex_color";
}

/// Bibliothèque de shaders du renderer : la maison des sources.
///
/// Les shaders intégrés du moteur vivent dans `chaos_renderer/shaders/*.wgsl`
/// (embarqués à la compilation) ; matériaux, post-process et jeux
/// enregistreront les leurs. Le futur shader compiler et l'asset pipeline
/// remplaceront le chargement, pas cette organisation.
#[derive(Debug, Default)]
pub struct ShaderLibrary {
    entries: HashMap<String, ShaderSource>,
}

impl ShaderLibrary {
    pub fn with_builtins() -> Self {
        let mut library = Self::default();
        library.register(
            builtin::VERTEX_COLOR,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/vertex_color.wgsl"))),
        );
        library
    }

    pub fn register(&mut self, name: impl Into<String>, source: ShaderSource) {
        let name = name.into();
        if self.entries.insert(name.clone(), source).is_some() {
            warn!("shader '{name}' replaced in the library");
        }
    }

    pub fn get(&self, name: &str) -> Option<&ShaderSource> {
        self.entries.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ShaderSource)> {
        self.entries
            .iter()
            .map(|(name, source)| (name.as_str(), source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_loaded() {
        let library = ShaderLibrary::with_builtins();
        assert!(library.contains(builtin::VERTEX_COLOR));
        assert!(library.iter().count() > 0);
    }

    #[test]
    fn register_then_get_roundtrip() {
        let mut library = ShaderLibrary::default();
        library.register("game.custom", ShaderSource::Wgsl(String::from("code")));
        assert_eq!(
            library.get("game.custom"),
            Some(&ShaderSource::Wgsl(String::from("code")))
        );
        assert!(library.get("game.unknown").is_none());
    }

    #[test]
    fn register_replaces_existing_entry() {
        let mut library = ShaderLibrary::default();
        library.register("game.custom", ShaderSource::Wgsl(String::from("v1")));
        library.register("game.custom", ShaderSource::Wgsl(String::from("v2")));
        assert_eq!(
            library.get("game.custom"),
            Some(&ShaderSource::Wgsl(String::from("v2")))
        );
    }
}
