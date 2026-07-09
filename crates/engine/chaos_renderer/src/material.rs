use chaos_core::Color;

use crate::resources::{MaterialBindingHandle, PipelineHandle, SamplerHandle, TextureHandle};

/// Identifiant opaque d'un material. Générationnel : un handle dont le
/// material a été détruit est détecté, jamais résolu vers un autre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Description d'un material — LE concept de surface du moteur : le
/// pipeline qui dessine, la couleur de base (paramètre uniform), et la
/// texture/le sampler optionnels (fallbacks builtin : texture blanche 1×1,
/// sampler Linear+Repeat). Immuable après création ; la mise à jour des
/// paramètres viendra avec ses besoins réels. Le futur PBR étendra les
/// paramètres (metallic, roughness…) sans changer le concept.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialDescriptor {
    pub label: String,
    pub pipeline: PipelineHandle,
    pub base_color: Color,
    pub texture: Option<TextureHandle>,
    pub sampler: Option<SamplerHandle>,
}

impl MaterialDescriptor {
    pub fn new(label: impl Into<String>, pipeline: PipelineHandle) -> Self {
        Self {
            label: label.into(),
            pipeline,
            base_color: Color::WHITE,
            texture: None,
            sampler: None,
        }
    }

    pub fn with_base_color(mut self, base_color: Color) -> Self {
        self.base_color = base_color;
        self
    }

    pub fn with_texture(mut self, texture: TextureHandle) -> Self {
        self.texture = Some(texture);
        self
    }

    pub fn with_sampler(mut self, sampler: SamplerHandle) -> Self {
        self.sampler = Some(sampler);
        self
    }
}

/// Ressource material côté renderer : le pipeline associé et le binding
/// GPU (groupe 2) possédé. La texture et le sampler référencés ne sont PAS
/// possédés — ils sont partageables entre materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterialRecord {
    pub(crate) pipeline: PipelineHandle,
    pub(crate) binding: MaterialBindingHandle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_white_and_untextured() {
        let descriptor = MaterialDescriptor::new("m", PipelineHandle(3));
        assert_eq!(descriptor.pipeline, PipelineHandle(3));
        assert_eq!(descriptor.base_color, Color::WHITE);
        assert!(descriptor.texture.is_none());
        assert!(descriptor.sampler.is_none());
    }

    #[test]
    fn builders_override_the_defaults() {
        let texture = TextureHandle {
            index: 4,
            generation: 1,
        };
        let sampler = SamplerHandle {
            index: 2,
            generation: 0,
        };
        let descriptor = MaterialDescriptor::new("m", PipelineHandle(0))
            .with_base_color(Color::rgb(0.5, 0.25, 1.0))
            .with_texture(texture)
            .with_sampler(sampler);
        assert_eq!(descriptor.base_color, Color::rgb(0.5, 0.25, 1.0));
        assert_eq!(descriptor.texture, Some(texture));
        assert_eq!(descriptor.sampler, Some(sampler));
    }
}
