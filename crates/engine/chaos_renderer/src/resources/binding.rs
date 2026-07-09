use chaos_core::Color;

use crate::resources::sampler::SamplerHandle;
use crate::resources::texture::TextureHandle;

/// Identifiant opaque du binding GPU d'un material. Générationnel : un
/// handle dont la ressource a été détruite est détecté, jamais résolu
/// ailleurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialBindingHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Description bas niveau du groupe(2) d'un material, dans le vocabulaire
/// que voit le trait backend : texture et sampler DÉJÀ résolus (le Renderer
/// a appliqué les fallbacks) + la couleur de base des MaterialUniforms.
/// Le concept haut niveau est `MaterialDescriptor` (`src/material.rs`).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialBindingDescriptor {
    pub label: String,
    pub texture: TextureHandle,
    pub sampler: SamplerHandle,
    pub base_color: Color,
}

impl MaterialBindingDescriptor {
    pub fn new(
        label: impl Into<String>,
        texture: TextureHandle,
        sampler: SamplerHandle,
        base_color: Color,
    ) -> Self {
        Self {
            label: label.into(),
            texture,
            sampler,
            base_color,
        }
    }
}
