use std::collections::HashMap;

use log::warn;

use crate::resources::ShaderSource;

/// Noms des shaders intégrés du moteur (namespace `chaos.`).
pub mod builtin {
    /// Le shader position + couleur interpolée (géométries colorées).
    pub const VERTEX_COLOR: &str = "chaos.vertex_color";
    /// Le shader texturé (UV + material : texture, sampler, teinte).
    pub const TEXTURED: &str = "chaos.textured";
    /// Le shader ÉCLAIRÉ (normale + UV + material) : ambiante + Lambert
    /// diffus depuis les lumières de la frame.
    pub const LIT: &str = "chaos.lit";
    /// Le shader PBR (Cook-Torrance GGX, metallic/roughness) : les cinq
    /// slots de textures material + paramètres, sous les lumières de la
    /// frame.
    pub const PBR: &str = "chaos.pbr";
    /// Le shader du CIEL : triangle plein écran à la profondeur maximale,
    /// qui échantillonne la cubemap d'environnement de la frame.
    pub const SKY: &str = "chaos.sky";
    /// Le shader de la passe d'OMBRE : profondeur seule dans le clip de
    /// la lumière — vertex uniquement, aucun étage fragment.
    pub const SHADOW: &str = "chaos.shadow";
    /// Le shader du DEBUG RENDERING : des lignes monde pré-transformées
    /// (position + couleur RGBA), projetées par la seule vue-projection.
    pub const DEBUG: &str = "chaos.debug";
}

/// Convention d'entrées des shaders — l'autorité UNIQUE des groupes et des
/// slots de binding. Les shaders déclarent, le moteur fournit : groupes 0/1
/// bindés automatiquement (uniforms de frame et d'objet), groupe 2 fourni
/// par le contenu (ressources material — texture + sampler aujourd'hui,
/// paramètres demain). Le backend et les WGSL intégrés s'y conforment —
/// verrouillé par le test naga `builtin_shaders_follow_the_input_conventions`.
pub mod inputs {
    /// Le groupe des uniforms de FRAME (vue-projection) — bindé par le moteur.
    pub const FRAME_GROUP: u32 = 0;
    /// Le slot des uniforms de frame dans son groupe.
    pub const FRAME_UNIFORMS_BINDING: u32 = 0;
    /// Le slot des lumières de la frame (ambiante + sources), même groupe.
    pub const FRAME_LIGHTS_BINDING: u32 = 1;
    /// Le slot de la cubemap d'environnement de la frame (texture_cube).
    pub const FRAME_ENVIRONMENT_BINDING: u32 = 2;
    /// Le slot du sampler d'environnement, même groupe.
    pub const FRAME_ENVIRONMENT_SAMPLER_BINDING: u32 = 3;
    /// Le slot de la shadow map directionnelle (texture_depth_2d).
    pub const FRAME_SHADOW_BINDING: u32 = 4;
    /// Le slot du sampler de COMPARAISON des ombres, même groupe.
    pub const FRAME_SHADOW_SAMPLER_BINDING: u32 = 5;
    /// Le groupe des uniforms d'OBJET (matrice modèle) — bindé par le moteur.
    pub const OBJECT_GROUP: u32 = 1;
    /// Le slot des uniforms d'objet dans son groupe.
    pub const OBJECT_UNIFORMS_BINDING: u32 = 0;
    /// La PREMIÈRE `@location` des attributs d'INSTANCE (matrice modèle
    /// 4..=7, matrice des normales 8..=11 — huit `Float32x4`) : au-dessus
    /// des attributs de tous les layouts de mesh builtin, consommée par
    /// les entrées `vs_instanced`. L'autorité du layout est
    /// `resources::instance_transforms_layout()`.
    pub const INSTANCE_LOCATION_BASE: u32 = 4;
    /// Le groupe MATERIAL — fourni par le contenu.
    pub const MATERIAL_GROUP: u32 = 2;
    /// Le slot de la texture du material.
    pub const MATERIAL_TEXTURE_BINDING: u32 = 0;
    /// Le slot du sampler du material.
    pub const MATERIAL_SAMPLER_BINDING: u32 = 1;
    /// Le slot des paramètres du material (base_color, …).
    pub const MATERIAL_UNIFORMS_BINDING: u32 = 2;
    /// Le slot de la texture metallic/roughness (G=roughness, B=metallic).
    pub const MATERIAL_METALLIC_ROUGHNESS_BINDING: u32 = 3;
    /// Le slot de la normal map (tangent-space, +Y vert).
    pub const MATERIAL_NORMAL_BINDING: u32 = 4;
    /// Le slot de la texture d'occlusion ambiante (canal R).
    pub const MATERIAL_OCCLUSION_BINDING: u32 = 5;
    /// Le slot de la texture émissive (sRGB).
    pub const MATERIAL_EMISSIVE_BINDING: u32 = 6;
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
    /// Bibliothèque pré-remplie des shaders intégrés du moteur
    /// (namespace `chaos.`).
    pub fn with_builtins() -> Self {
        let mut library = Self::default();
        library.register(
            builtin::VERTEX_COLOR,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/vertex_color.wgsl"))),
        );
        library.register(
            builtin::TEXTURED,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/textured.wgsl"))),
        );
        library.register(
            builtin::LIT,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/lit.wgsl"))),
        );
        library.register(
            builtin::PBR,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/pbr.wgsl"))),
        );
        library.register(
            builtin::SKY,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/sky.wgsl"))),
        );
        library.register(
            builtin::SHADOW,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/shadow.wgsl"))),
        );
        library.register(
            builtin::DEBUG,
            ShaderSource::Wgsl(String::from(include_str!("../shaders/debug.wgsl"))),
        );
        library
    }

    /// Enregistre (ou remplace, avec `warn!`) une source sous son nom
    /// logique — le point d'extension des materials et jeux.
    pub fn register(&mut self, name: impl Into<String>, source: ShaderSource) {
        let name = name.into();
        if self.entries.insert(name.clone(), source).is_some() {
            warn!("shader '{name}' replaced in the library");
        }
    }

    /// La source enregistrée sous ce nom, si elle existe.
    pub fn get(&self, name: &str) -> Option<&ShaderSource> {
        self.entries.get(name)
    }

    /// Le nom est-il enregistré ?
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Itère les paires (nom, source) — listage et diagnostics.
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
