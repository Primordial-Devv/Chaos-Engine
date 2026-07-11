/// Source d'un shader, dans le vocabulaire du moteur.
///
/// WGSL est le langage shader officiel de Chaos Engine (compilable vers
/// SPIR-V via naga pour un futur backend maison) ; d'autres formats
/// pourront s'ajouter ici sans toucher au reste du moteur.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShaderSource {
    /// Une source WGSL complète.
    Wgsl(String),
}

/// Référence à un shader dans un descripteur : par nom (résolu via la
/// bibliothèque du renderer) ou source directe (prototypage, tests).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShaderRef {
    /// Un nom logique, résolu via la `ShaderLibrary`.
    Named(String),
    /// Une source directe, sans passer par la bibliothèque.
    Inline(ShaderSource),
}

impl From<&str> for ShaderRef {
    fn from(name: &str) -> Self {
        Self::Named(String::from(name))
    }
}

impl From<String> for ShaderRef {
    fn from(name: String) -> Self {
        Self::Named(name)
    }
}

impl From<ShaderSource> for ShaderRef {
    fn from(source: ShaderSource) -> Self {
        Self::Inline(source)
    }
}
