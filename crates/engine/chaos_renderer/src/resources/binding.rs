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

/// Les PARAMÈTRES uniformes d'un material — le contenu du buffer du
/// groupe(2) binding(2), 48 octets côté GPU : base_color, (metallic,
/// roughness), émissif. `Default` = les défauts du moteur (un
/// diélectrique mat, éteint) — LA référence unique du « hors défaut ».
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParams {
    /// La couleur de base (teinte multiplicative, alpha compris).
    pub base_color: Color,
    /// Le facteur métallique (0 = diélectrique).
    pub metallic: f32,
    /// Le facteur de rugosité (1 = mat).
    pub roughness: f32,
    /// La surface reçoit-elle les ombres ? (1.0 côté GPU quand vrai.)
    pub receive_shadows: bool,
    /// Le seuil d'élimination des fragments (materials `Masked`) —
    /// consommé par l'entrée `fs_masked` seulement.
    pub alpha_cutoff: f32,
    /// La couleur émissive (noir = éteint) — l'alpha est ignoré.
    pub emissive: Color,
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            metallic: 0.0,
            roughness: 1.0,
            receive_shadows: true,
            alpha_cutoff: 0.5,
            emissive: Color::BLACK,
        }
    }
}

/// Description bas niveau du groupe(2) d'un material, dans le vocabulaire
/// que voit le trait backend — les SLOTS FIXES, TOUJOURS remplis : les
/// cinq textures et le sampler arrivent DÉJÀ résolus (le Renderer a
/// appliqué les fallbacks neutres — blanche pour base/metallic-roughness/
/// occlusion/émissif, normale plate pour la normal map), jamais
/// optionnels ici. Le concept haut niveau est `MaterialDescriptor`
/// (`src/material.rs`).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialBindingDescriptor {
    /// Le label de diagnostic.
    pub label: String,
    /// La texture de base (albédo), résolue.
    pub texture: TextureHandle,
    /// La texture metallic/roughness (G=roughness, B=metallic), résolue.
    pub metallic_roughness_texture: TextureHandle,
    /// La normal map (tangent-space), résolue.
    pub normal_map: TextureHandle,
    /// La texture d'occlusion (canal R), résolue.
    pub occlusion_texture: TextureHandle,
    /// La texture émissive, résolue.
    pub emissive_texture: TextureHandle,
    /// Le sampler partagé par TOUTES les textures du material (V1).
    pub sampler: SamplerHandle,
    /// Les paramètres uniformes (48 octets côté GPU).
    pub params: MaterialParams,
}
