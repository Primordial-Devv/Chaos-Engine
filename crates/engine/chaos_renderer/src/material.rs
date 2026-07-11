use chaos_core::Color;

use crate::resources::{
    ColorVertex, LitVertex, MaterialBindingHandle, SamplerHandle, ShaderRef, TextureHandle,
    TexturedVertex, VertexLayout,
};
use crate::shaders::builtin;

/// Identifiant opaque d'un material. Générationnel : un handle dont le
/// material a été détruit est détecté, jamais résolu vers un autre.
/// L'identité SURVIT aux mises à jour (`set_material_*`) — c'est le même
/// material qui change d'apparence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Le MODÈLE d'un material : sa famille de shaders, le vertex layout
/// qu'il attend et son contrat d'entrées. Le pipeline concret n'est plus
/// l'affaire du consommateur — le renderer résout (modèle + état +
/// format de la destination de passe) vers une permutation de pipeline,
/// dédupliquée par cache. Les futurs modèles éclairés (`Lit`, `Pbr`)
/// seront des variantes de plus.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MaterialModel {
    /// La couleur vient des sommets (`chaos.vertex_color`, layout
    /// `ColorVertex`) — AUCUNE entrée material : ni texture, ni
    /// `base_color` (les refus sont explicites).
    VertexColor,
    /// Non éclairé, texturé et teinté (`chaos.textured`, layout
    /// `TexturedVertex`) : `sample(texture) × base_color`.
    Unlit,
    /// ÉCLAIRÉ (`chaos.lit`, layout `LitVertex` — position + normale +
    /// UV) : `sample × base_color × (ambiante + Lambert diffus)` depuis
    /// les lumières de la frame.
    Lit,
    /// PHYSIQUEMENT PLAUSIBLE (`chaos.pbr`, layout `LitVertex`) :
    /// Cook-Torrance GGX en metallic/roughness — base color, metallic,
    /// roughness, normal map, occlusion ambiante et émissif, chacun en
    /// constante et/ou texture (conventions glTF : voir la section
    /// « Le matériau PBR » d'overview.md). Les tangentes sont DÉRIVÉES à
    /// l'écran — aucun attribut de vertex supplémentaire.
    Pbr,
    /// Un shader d'application (nommé dans la bibliothèque ou inline) —
    /// le layout attendu est déclaré, `material_inputs` dit si le shader
    /// lit le groupe(2) (texture + sampler + `MaterialUniforms`).
    Custom {
        /// Le shader de la famille.
        shader: ShaderRef,
        /// Le vertex layout que le shader attend.
        vertex_layout: VertexLayout,
        /// Le shader déclare-t-il les entrées material (groupe 2) ?
        material_inputs: bool,
    },
}

impl MaterialModel {
    /// Le vertex layout que les meshes dessinés avec ce modèle doivent
    /// porter — la validation écarte les draws désassortis.
    pub fn expected_vertex_layout(&self) -> VertexLayout {
        match self {
            Self::VertexColor => ColorVertex::layout(),
            Self::Unlit => TexturedVertex::layout(),
            Self::Lit | Self::Pbr => LitVertex::layout(),
            Self::Custom { vertex_layout, .. } => vertex_layout.clone(),
        }
    }

    /// La référence shader de la famille.
    pub fn shader_ref(&self) -> ShaderRef {
        match self {
            Self::VertexColor => ShaderRef::Named(String::from(builtin::VERTEX_COLOR)),
            Self::Unlit => ShaderRef::Named(String::from(builtin::TEXTURED)),
            Self::Lit => ShaderRef::Named(String::from(builtin::LIT)),
            Self::Pbr => ShaderRef::Named(String::from(builtin::PBR)),
            Self::Custom { shader, .. } => shader.clone(),
        }
    }

    /// Le modèle consomme-t-il les entrées material (groupe 2) ?
    pub fn material_inputs(&self) -> bool {
        match self {
            Self::VertexColor => false,
            Self::Unlit | Self::Lit | Self::Pbr => true,
            Self::Custom {
                material_inputs, ..
            } => *material_inputs,
        }
    }

    /// Le modèle consomme-t-il les PROPRIÉTÉS PBR (metallic, roughness,
    /// normal map, occlusion, émissif) ? Distinct de `material_inputs` :
    /// `Unlit`/`Lit` lisent le groupe(2) mais ignorent ces propriétés —
    /// les fournir y serait inerte, donc refusé. Un `Custom` à entrées
    /// material voit TOUT le groupe(2) : la responsabilité de les lire
    /// appartient à son shader (délégation documentée).
    pub fn pbr_inputs(&self) -> bool {
        match self {
            Self::Pbr => true,
            Self::Custom {
                material_inputs, ..
            } => *material_inputs,
            Self::VertexColor | Self::Unlit | Self::Lit => false,
        }
    }

    /// Le modèle RÉAGIT-IL à l'éclairage de la frame ? C'est la condition
    /// pour RECEVOIR des ombres — un modèle non éclairé n'a aucune
    /// contribution directe à atténuer, `receive_shadows` hors défaut y
    /// serait inerte, donc refusé. Un `Custom` à entrées material délègue
    /// à son shader (le patron de `pbr_inputs`).
    pub fn lighting_inputs(&self) -> bool {
        match self {
            Self::Lit | Self::Pbr => true,
            Self::Custom {
                material_inputs, ..
            } => *material_inputs,
            Self::VertexColor | Self::Unlit => false,
        }
    }

    /// L'étiquette du modèle dans les labels de pipelines générés.
    pub(crate) fn tag(&self) -> String {
        match self {
            Self::VertexColor => String::from("vertex_color"),
            Self::Unlit => String::from("unlit"),
            Self::Lit => String::from("lit"),
            Self::Pbr => String::from("pbr"),
            Self::Custom { shader, .. } => match shader {
                ShaderRef::Named(name) => format!("custom.{name}"),
                ShaderRef::Inline(_) => String::from("custom.inline"),
            },
        }
    }
}

/// Le niveau d'opacité d'un material — déclaré, inspectable, et
/// l'AUTORITÉ UNIQUE des contrats de rendu par catégorie : la
/// permutation de pipeline, la partition de la passe (opaques → masked
/// → ciel → transparents triés) et la collecte des casters d'ombre
/// consomment ses méthodes, jamais des règles locales. Aligné sur les
/// `alphaMode` de glTF (OPAQUE, MASK, BLEND).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MaterialOpacity {
    /// Surface pleine : blend REPLACE, écrit la profondeur, projette.
    #[default]
    Opaque,
    /// Surface AJOURÉE (alpha cutout — grilles, feuillages) : blend
    /// REPLACE, écrit la profondeur, projette (silhouette PLEINE en V1,
    /// trous compris — les casters alpha-testés sont l'extension
    /// notée) ; le fragment sous `alpha_cutoff` est ÉLIMINÉ (`discard`,
    /// entrée `fs_masked` — le `alphaMode: MASK` de glTF).
    Masked,
    /// Surface translucide : alpha blending, profondeur en lecture
    /// seule, ne projette jamais, rendue APRÈS les opaques de sa passe
    /// et TRIÉE par profondeur (arrière → avant).
    Transparent,
}

impl MaterialOpacity {
    /// La catégorie écrit-elle la profondeur ? `Transparent` teste sans
    /// écrire (le couplage V1 : une surface translucide ne doit pas
    /// occulter).
    pub fn writes_depth(&self) -> bool {
        !matches!(self, Self::Transparent)
    }

    /// La catégorie dessine-t-elle en alpha blending ? (Sinon REPLACE.)
    pub fn blends(&self) -> bool {
        matches!(self, Self::Transparent)
    }

    /// La catégorie PROJETTE-t-elle des ombres ? Opaque et Masked oui —
    /// masked en silhouette pleine V1 (le shader d'ombre ne lit aucun
    /// material) ; Transparent jamais.
    pub fn casts_shadows(&self) -> bool {
        !matches!(self, Self::Transparent)
    }

    /// Le point d'entrée fragment des permutations de la catégorie —
    /// `fs_masked` élimine sous le cutoff, les autres entrées restent
    /// SANS `discard` (l'early-Z des opaques est préservé).
    pub(crate) fn fragment_entry(&self) -> &'static str {
        match self {
            Self::Masked => "fs_masked",
            Self::Opaque | Self::Transparent => "fs_main",
        }
    }

    /// Le suffixe de la catégorie dans les labels de pipelines générés
    /// — vide pour le défaut.
    pub(crate) fn label_suffix(&self) -> &'static str {
        match self {
            Self::Opaque => "",
            Self::Masked => ".masked",
            Self::Transparent => ".transparent",
        }
    }
}

/// Description d'un material — LA couche visuelle du moteur : un modèle
/// (famille de shaders), des paramètres (`base_color`), des textures
/// (fallbacks builtin : blanche 1×1, sampler Linear+Repeat), un état de
/// rendu (`double_sided`) et un niveau d'opacité. Aucun pipeline : le
/// renderer résout les permutations en interne, par passe. Modèle et
/// état sont figés à la création (recréer le material pour en changer) ;
/// paramètres et textures se modifient par `set_material_*`.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialDescriptor {
    /// Le label de diagnostic — l'identité humaine du material.
    pub label: String,
    /// Le modèle : la famille de shaders et son contrat.
    pub model: MaterialModel,
    /// La couleur de base (paramètre uniform, teinte multiplicative) —
    /// refusée hors défaut si le modèle n'a pas d'entrées material.
    pub base_color: Color,
    /// La texture échantillonnée ; `None` → texture blanche builtin.
    /// Refusée si le modèle n'a pas d'entrées material.
    pub texture: Option<TextureHandle>,
    /// Le sampler de lecture (partagé par TOUTES les textures du
    /// material en V1) ; `None` → sampler builtin Linear+Repeat.
    /// Refusé si le modèle n'a pas d'entrées material.
    pub sampler: Option<SamplerHandle>,
    /// `false` (défaut) : faces arrière cullées — le réglage 3D opaque
    /// standard ; `true` : les deux faces dessinées (quads, feuillages).
    pub double_sided: bool,
    /// Le niveau d'opacité (défaut `Opaque`).
    pub opacity: MaterialOpacity,
    /// Le facteur métallique (défaut 0.0 — diélectrique). Multiplié par
    /// le canal B de la texture metallic/roughness (convention glTF).
    /// Propriété PBR : refusée hors défaut si `pbr_inputs()` est faux.
    pub metallic: f32,
    /// Le facteur de rugosité (défaut 1.0 — mat). Multiplié par le canal
    /// G de la texture metallic/roughness (convention glTF). Propriété
    /// PBR.
    pub roughness: f32,
    /// La texture metallic/roughness (LINÉAIRE, G=roughness, B=metallic
    /// — le packing glTF) ; `None` → blanche builtin (les facteurs
    /// passent tels quels). Propriété PBR, FIGÉE après création.
    pub metallic_roughness_texture: Option<TextureHandle>,
    /// La normal map (LINÉAIRE, tangent-space, +Y vert — glTF) ; `None`
    /// → normale plate builtin. Propriété PBR, FIGÉE après création.
    pub normal_map: Option<TextureHandle>,
    /// La texture d'occlusion ambiante (LINÉAIRE, canal R) ; `None` →
    /// blanche builtin (pas d'occlusion). Propriété PBR, FIGÉE après
    /// création.
    pub occlusion_texture: Option<TextureHandle>,
    /// La couleur émissive (défaut noir — éteint), multipliée par la
    /// texture émissive. Propriété PBR, modifiable par
    /// `set_material_emissive`.
    pub emissive: Color,
    /// La texture émissive (sRGB) ; `None` → blanche builtin (le facteur
    /// passe tel quel). Propriété PBR, FIGÉE après création.
    pub emissive_texture: Option<TextureHandle>,
    /// `true` (défaut) : les draws opaques de ce material PROJETTENT des
    /// ombres. État de rendu, FIGÉ à la création. Un material transparent
    /// ne projette jamais en V1, quel que soit ce flag.
    pub cast_shadows: bool,
    /// `true` (défaut) : les surfaces de ce material REÇOIVENT les ombres
    /// (la contribution directe de la lumière projetante est atténuée).
    /// État de rendu, FIGÉ à la création — refusé hors défaut si le
    /// modèle ne réagit pas à l'éclairage (`lighting_inputs()`).
    pub receive_shadows: bool,
    /// Le seuil d'élimination des fragments d'un material `Masked`
    /// (alpha du sample × base_color < cutoff → éliminé), dans [0, 1] —
    /// défaut 0.5, la convention glTF. Réservé à `Masked` : hors défaut
    /// sur une autre opacité, la création est refusée en nommant la
    /// propriété. Modifiable par `set_material_alpha_cutoff`.
    pub alpha_cutoff: f32,
    /// `true` (défaut) : les draws de ce material sont soumis au
    /// FRUSTUM CULLING (rejetés hors champ). `false` = FORCÉ VISIBLE —
    /// jamais cullé, ni par la caméra de la passe ni par le volume de
    /// lumière (les fonds plein écran, les objets à géométrie déformée
    /// par shader dont les bounds mentent). État de rendu, FIGÉ à la
    /// création.
    pub frustum_culled: bool,
}

impl MaterialDescriptor {
    /// Descripteur aux défauts du moteur : couleur blanche, textures
    /// builtin, faces arrière cullées, opaque.
    pub fn new(label: impl Into<String>, model: MaterialModel) -> Self {
        Self {
            label: label.into(),
            model,
            base_color: Color::WHITE,
            texture: None,
            sampler: None,
            double_sided: false,
            opacity: MaterialOpacity::Opaque,
            metallic: 0.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_map: None,
            occlusion_texture: None,
            emissive: Color::BLACK,
            emissive_texture: None,
            cast_shadows: true,
            receive_shadows: true,
            alpha_cutoff: 0.5,
            frustum_culled: true,
        }
    }

    /// Force ce material VISIBLE : ses draws ne sont jamais cullés.
    pub fn without_frustum_culling(mut self) -> Self {
        self.frustum_culled = false;
        self
    }

    /// Fixe le seuil d'élimination des fragments (materials `Masked`).
    pub fn with_alpha_cutoff(mut self, alpha_cutoff: f32) -> Self {
        self.alpha_cutoff = alpha_cutoff;
        self
    }

    /// Retire ce material des PROJETEURS d'ombres.
    pub fn without_shadow_cast(mut self) -> Self {
        self.cast_shadows = false;
        self
    }

    /// Retire ce material des RECEVEURS d'ombres.
    pub fn without_shadow_receive(mut self) -> Self {
        self.receive_shadows = false;
        self
    }

    /// Fixe la couleur de base.
    pub fn with_base_color(mut self, base_color: Color) -> Self {
        self.base_color = base_color;
        self
    }

    /// Attache une texture.
    pub fn with_texture(mut self, texture: TextureHandle) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Attache un sampler.
    pub fn with_sampler(mut self, sampler: SamplerHandle) -> Self {
        self.sampler = Some(sampler);
        self
    }

    /// Dessine les deux faces (désactive le culling).
    pub fn double_sided(mut self) -> Self {
        self.double_sided = true;
        self
    }

    /// Fixe le niveau d'opacité.
    pub fn with_opacity(mut self, opacity: MaterialOpacity) -> Self {
        self.opacity = opacity;
        self
    }

    /// Fixe le facteur métallique (0 = diélectrique, 1 = métal).
    pub fn with_metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic;
        self
    }

    /// Fixe le facteur de rugosité (0 = miroir, 1 = mat).
    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }

    /// Attache la texture metallic/roughness (G=roughness, B=metallic).
    pub fn with_metallic_roughness_texture(mut self, texture: TextureHandle) -> Self {
        self.metallic_roughness_texture = Some(texture);
        self
    }

    /// Attache la normal map (tangent-space, +Y vert).
    pub fn with_normal_map(mut self, texture: TextureHandle) -> Self {
        self.normal_map = Some(texture);
        self
    }

    /// Attache la texture d'occlusion ambiante (canal R).
    pub fn with_occlusion_texture(mut self, texture: TextureHandle) -> Self {
        self.occlusion_texture = Some(texture);
        self
    }

    /// Fixe la couleur émissive (noir = éteint).
    pub fn with_emissive(mut self, emissive: Color) -> Self {
        self.emissive = emissive;
        self
    }

    /// Attache la texture émissive (sRGB).
    pub fn with_emissive_texture(mut self, texture: TextureHandle) -> Self {
        self.emissive_texture = Some(texture);
        self
    }

    /// Une propriété PBR est-elle posée hors défaut ? La matière du refus
    /// sur les modèles qui ne les consomment pas — nommée par champ.
    pub(crate) fn first_pbr_property(&self) -> Option<&'static str> {
        if self.metallic != 0.0 {
            return Some("metallic");
        }
        if self.roughness != 1.0 {
            return Some("roughness");
        }
        if self.metallic_roughness_texture.is_some() {
            return Some("metallic_roughness_texture");
        }
        if self.normal_map.is_some() {
            return Some("normal_map");
        }
        if self.occlusion_texture.is_some() {
            return Some("occlusion_texture");
        }
        if self.emissive != Color::BLACK {
            return Some("emissive");
        }
        if self.emissive_texture.is_some() {
            return Some("emissive_texture");
        }
        None
    }
}

/// La photo d'un material vivant (`Renderer::material_info`) — tout ce
/// que le futur éditeur inspecte : identité, modèle, paramètres,
/// ressources RÉSOLUES (fallbacks appliqués), état de rendu. Reflète les
/// mises à jour.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialInfo {
    /// Le label du material.
    pub label: String,
    /// Son modèle.
    pub model: MaterialModel,
    /// Sa couleur de base courante.
    pub base_color: Color,
    /// Sa texture résolue (le fallback builtin si aucune fournie).
    pub texture: TextureHandle,
    /// Son sampler résolu.
    pub sampler: SamplerHandle,
    /// Les deux faces sont-elles dessinées ?
    pub double_sided: bool,
    /// Son niveau d'opacité.
    pub opacity: MaterialOpacity,
    /// Son facteur métallique courant.
    pub metallic: f32,
    /// Son facteur de rugosité courant.
    pub roughness: f32,
    /// Sa texture metallic/roughness résolue.
    pub metallic_roughness_texture: TextureHandle,
    /// Sa normal map résolue.
    pub normal_map: TextureHandle,
    /// Sa texture d'occlusion résolue.
    pub occlusion_texture: TextureHandle,
    /// Sa couleur émissive courante.
    pub emissive: Color,
    /// Sa texture émissive résolue.
    pub emissive_texture: TextureHandle,
    /// Ses draws opaques projettent-ils des ombres ?
    pub cast_shadows: bool,
    /// Ses surfaces reçoivent-elles les ombres ?
    pub receive_shadows: bool,
    /// Son seuil d'élimination courant (materials `Masked`).
    pub alpha_cutoff: f32,
    /// Ses draws sont-ils soumis au frustum culling ?
    pub frustum_culled: bool,
}

/// Ressource material côté renderer : la description RETENUE (modèle,
/// paramètres, état — l'inspection et la résolution de pipeline la
/// relisent), le binding GPU (groupe 2) possédé, et la texture/le
/// sampler PARTAGÉS (parts comptées par le registre de durée de vie).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MaterialRecord {
    pub(crate) label: String,
    pub(crate) model: MaterialModel,
    pub(crate) base_color: Color,
    pub(crate) binding: MaterialBindingHandle,
    pub(crate) texture: TextureHandle,
    pub(crate) sampler: SamplerHandle,
    pub(crate) double_sided: bool,
    pub(crate) opacity: MaterialOpacity,
    pub(crate) metallic: f32,
    pub(crate) roughness: f32,
    pub(crate) metallic_roughness_texture: TextureHandle,
    pub(crate) normal_map: TextureHandle,
    pub(crate) occlusion_texture: TextureHandle,
    pub(crate) emissive: Color,
    pub(crate) emissive_texture: TextureHandle,
    pub(crate) cast_shadows: bool,
    pub(crate) receive_shadows: bool,
    pub(crate) alpha_cutoff: f32,
    pub(crate) frustum_culled: bool,
}

impl MaterialRecord {
    /// Les CINQ textures du material, dans l'ordre des slots — la
    /// monnaie des refcounts et du check de feedback.
    pub(crate) fn textures(&self) -> [TextureHandle; 5] {
        [
            self.texture,
            self.metallic_roughness_texture,
            self.normal_map,
            self.occlusion_texture,
            self.emissive_texture,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_white_opaque_and_culled() {
        let descriptor = MaterialDescriptor::new("m", MaterialModel::Unlit);
        assert_eq!(descriptor.model, MaterialModel::Unlit);
        assert_eq!(descriptor.base_color, Color::WHITE);
        assert!(descriptor.texture.is_none());
        assert!(descriptor.sampler.is_none());
        assert!(!descriptor.double_sided);
        assert_eq!(descriptor.opacity, MaterialOpacity::Opaque);
        // Les défauts PBR : un diélectrique mat, éteint — aucune
        // propriété posée.
        assert_eq!(descriptor.metallic, 0.0);
        assert_eq!(descriptor.roughness, 1.0);
        assert!(descriptor.metallic_roughness_texture.is_none());
        assert!(descriptor.normal_map.is_none());
        assert!(descriptor.occlusion_texture.is_none());
        assert_eq!(descriptor.emissive, Color::BLACK);
        assert!(descriptor.emissive_texture.is_none());
        assert!(descriptor.first_pbr_property().is_none());
        assert!(descriptor.cast_shadows);
        assert!(descriptor.receive_shadows);
    }

    #[test]
    fn the_opacity_categories_declare_their_contracts() {
        // L'autorité unique des contrats de rendu par catégorie —
        // pipeline, partition et ombres la consomment, jamais des
        // règles locales.
        assert!(MaterialOpacity::Opaque.writes_depth());
        assert!(MaterialOpacity::Masked.writes_depth());
        assert!(!MaterialOpacity::Transparent.writes_depth());
        assert!(!MaterialOpacity::Opaque.blends());
        assert!(!MaterialOpacity::Masked.blends());
        assert!(MaterialOpacity::Transparent.blends());
        assert!(MaterialOpacity::Opaque.casts_shadows());
        assert!(MaterialOpacity::Masked.casts_shadows());
        assert!(!MaterialOpacity::Transparent.casts_shadows());
        assert_eq!(MaterialOpacity::Opaque.fragment_entry(), "fs_main");
        assert_eq!(MaterialOpacity::Masked.fragment_entry(), "fs_masked");
        assert_eq!(MaterialOpacity::Transparent.fragment_entry(), "fs_main");
        assert_eq!(MaterialOpacity::Opaque.label_suffix(), "");
        assert_eq!(MaterialOpacity::Masked.label_suffix(), ".masked");
        assert_eq!(MaterialOpacity::Transparent.label_suffix(), ".transparent");
    }

    #[test]
    fn the_alpha_cutoff_defaults_to_the_gltf_convention() {
        let descriptor = MaterialDescriptor::new("m", MaterialModel::Lit);
        assert_eq!(descriptor.alpha_cutoff, 0.5);
        let sparse = MaterialDescriptor::new("m", MaterialModel::Lit)
            .with_opacity(MaterialOpacity::Masked)
            .with_alpha_cutoff(0.25);
        assert_eq!(sparse.alpha_cutoff, 0.25);
        assert_eq!(sparse.opacity, MaterialOpacity::Masked);
    }

    #[test]
    fn shadow_flags_default_on_and_flip_off() {
        let caster_only = MaterialDescriptor::new("m", MaterialModel::Lit).without_shadow_receive();
        assert!(caster_only.cast_shadows);
        assert!(!caster_only.receive_shadows);
        let receiver_only = MaterialDescriptor::new("m", MaterialModel::Lit).without_shadow_cast();
        assert!(!receiver_only.cast_shadows);
        assert!(receiver_only.receive_shadows);
    }

    #[test]
    fn the_lit_models_declare_their_lighting_inputs() {
        assert!(MaterialModel::Lit.lighting_inputs());
        assert!(MaterialModel::Pbr.lighting_inputs());
        assert!(!MaterialModel::VertexColor.lighting_inputs());
        assert!(!MaterialModel::Unlit.lighting_inputs());
        let delegated = MaterialModel::Custom {
            shader: ShaderRef::from("game.toon"),
            vertex_layout: TexturedVertex::layout(),
            material_inputs: true,
        };
        assert!(delegated.lighting_inputs());
        let sealed = MaterialModel::Custom {
            shader: ShaderRef::from("game.flat"),
            vertex_layout: TexturedVertex::layout(),
            material_inputs: false,
        };
        assert!(!sealed.lighting_inputs());
    }

    #[test]
    fn pbr_builders_flag_their_property() {
        let texture = TextureHandle {
            index: 1,
            generation: 0,
        };
        let metallic = MaterialDescriptor::new("m", MaterialModel::Pbr).with_metallic(0.8);
        assert_eq!(metallic.first_pbr_property(), Some("metallic"));
        let rough = MaterialDescriptor::new("m", MaterialModel::Pbr).with_roughness(0.2);
        assert_eq!(rough.first_pbr_property(), Some("roughness"));
        let mapped = MaterialDescriptor::new("m", MaterialModel::Pbr).with_normal_map(texture);
        assert_eq!(mapped.first_pbr_property(), Some("normal_map"));
        let glowing = MaterialDescriptor::new("m", MaterialModel::Pbr)
            .with_emissive(Color::rgb(1.0, 0.5, 0.0));
        assert_eq!(glowing.first_pbr_property(), Some("emissive"));
        assert_eq!(glowing.emissive, Color::rgb(1.0, 0.5, 0.0));
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
        let descriptor = MaterialDescriptor::new("m", MaterialModel::Unlit)
            .with_base_color(Color::rgb(0.5, 0.25, 1.0))
            .with_texture(texture)
            .with_sampler(sampler)
            .double_sided()
            .with_opacity(MaterialOpacity::Transparent);
        assert_eq!(descriptor.base_color, Color::rgb(0.5, 0.25, 1.0));
        assert_eq!(descriptor.texture, Some(texture));
        assert_eq!(descriptor.sampler, Some(sampler));
        assert!(descriptor.double_sided);
        assert_eq!(descriptor.opacity, MaterialOpacity::Transparent);
    }

    #[test]
    fn the_models_declare_their_contracts() {
        assert_eq!(
            MaterialModel::VertexColor.expected_vertex_layout(),
            ColorVertex::layout()
        );
        assert!(!MaterialModel::VertexColor.material_inputs());
        assert_eq!(
            MaterialModel::Unlit.expected_vertex_layout(),
            TexturedVertex::layout()
        );
        assert!(MaterialModel::Unlit.material_inputs());
        assert_eq!(
            MaterialModel::Lit.expected_vertex_layout(),
            LitVertex::layout()
        );
        assert!(MaterialModel::Lit.material_inputs());
        assert_eq!(
            MaterialModel::Lit.shader_ref(),
            ShaderRef::Named(String::from("chaos.lit"))
        );
        assert_eq!(
            MaterialModel::Pbr.expected_vertex_layout(),
            LitVertex::layout()
        );
        assert!(MaterialModel::Pbr.material_inputs());
        assert!(MaterialModel::Pbr.pbr_inputs());
        assert!(!MaterialModel::Lit.pbr_inputs());
        assert!(!MaterialModel::Unlit.pbr_inputs());
        assert_eq!(
            MaterialModel::Pbr.shader_ref(),
            ShaderRef::Named(String::from("chaos.pbr"))
        );
        let custom = MaterialModel::Custom {
            shader: ShaderRef::from("game.toon"),
            vertex_layout: TexturedVertex::layout(),
            material_inputs: true,
        };
        assert_eq!(custom.expected_vertex_layout(), TexturedVertex::layout());
        assert!(custom.material_inputs());
        assert_eq!(
            custom.shader_ref(),
            ShaderRef::Named(String::from("game.toon"))
        );
    }
}
