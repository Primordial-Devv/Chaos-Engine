use chaos_core::Transform;
use chaos_core::math::{Mat4, Vec3};

use crate::light::FrameLights;
use crate::material::MaterialHandle;
use crate::mesh::MeshHandle;
use crate::pass::PassLoad;
use crate::resources::{
    BufferHandle, DebugVertex, MaterialBindingHandle, PipelineHandle, RenderTargetHandle,
};

/// La destination d'une passe de rendu : la surface de la fenêtre (avec
/// présentation) ou une cible hors écran (sans présentation) — le rendu
/// fenêtre est UN cas de destination parmi d'autres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderDestination {
    /// La surface de présentation — le chemin classique.
    #[default]
    Surface,
    /// Une render target hors écran — rendue puis exploitable comme
    /// texture, jamais présentée.
    Target(RenderTargetHandle),
}

/// Ordre de dessin public — le triplet du moteur : un mesh, un material,
/// une transformation. Le renderer résout material → (pipeline, binding),
/// mesh → buffers et transform → matrice modèle au moment de construire le
/// plan de frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawCommand {
    /// La géométrie à dessiner.
    pub mesh: MeshHandle,
    /// L'apparence : le material (pipeline + couleur + texture + sampler).
    pub material: MaterialHandle,
    /// La pose monde de l'objet (résolue en matrice modèle).
    pub transform: Transform,
}

/// La plage d'INSTANCES d'un draw instancié — des indices dans les
/// `InstanceTransforms` de SA passe (`FramePass.instances` ou
/// `FrameShadowPass.instances`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceRange {
    /// L'index de la première instance dans le tableau de la passe.
    pub first: u32,
    /// Le nombre d'instances (toujours ≥ 2 — un run de 1 reste un draw
    /// classique).
    pub count: u32,
}

/// Les transforms d'UNE instance — le miroir des `ObjectUniforms` en
/// données par instance : la matrice modèle et la matrice des normales,
/// packées par le backend en 128 octets (le stride du layout d'instance).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceTransforms {
    /// La matrice modèle de l'instance.
    pub model: Mat4,
    /// Sa matrice de transformation des normales (inverse-transposée).
    pub normal: Mat4,
}

/// Draw résolu du plan de frame — le vocabulaire buffers + matrices consommé
/// par le backend. `index_buffer` présent → rendu indexé (indices u16) ;
/// `element_count` compte les indices si indexé, les sommets sinon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameDraw {
    /// Le pipeline résolu du material.
    pub pipeline: PipelineHandle,
    /// Le vertex buffer du mesh ; `None` pour les sommets générés.
    pub vertex_buffer: Option<BufferHandle>,
    /// L'index buffer si le rendu est indexé (indices u16).
    pub index_buffer: Option<BufferHandle>,
    /// Le nombre d'indices si indexé, de sommets sinon.
    pub element_count: u32,
    /// La matrice modèle de l'objet — celle de la PREMIÈRE instance pour
    /// un draw instancié (le slot d'uniform reste écrit, non lu).
    pub model: Mat4,
    /// La matrice de transformation des normales (inverse-transposée du
    /// modèle — l'échelle non uniforme préservée).
    pub normal: Mat4,
    /// Le binding material (texture + sampler + uniforms) si le pipeline
    /// en consomme un.
    pub binding: Option<MaterialBindingHandle>,
    /// La plage d'instances d'un draw INSTANCIÉ (le pipeline est alors
    /// une permutation `vs_instanced`) ; `None` = le chemin classique,
    /// `ObjectUniforms` par draw.
    pub instances: Option<InstanceRange>,
}

/// Un BATCH de debug résolu d'une passe : une plage de sommets LIGNES
/// (des paires — topologie LineList) dans `FramePass.debug_vertices` et
/// le pipeline debug de son mode de profondeur. Encodé APRÈS les draws
/// de la passe — le slot réservé depuis la sous-phase transparence. Au
/// plus DEUX par passe : le batch testé (Scene) puis l'overlay, qui
/// dessine par-dessus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDebugBatch {
    /// Le pipeline debug résolu (lignes, blend alpha, profondeur en
    /// LECTURE SEULE — testée ou ignorée selon le mode).
    pub pipeline: PipelineHandle,
    /// L'index du premier sommet du batch dans `debug_vertices`.
    pub first_vertex: u32,
    /// Le nombre de sommets du batch (pair — des segments).
    pub vertex_count: u32,
}

/// Une passe RÉSOLUE du plan de frame — l'unité d'exécution du backend :
/// une destination, un traitement d'entrée, une caméra, des draws déjà
/// en ordre de rendu. Le backend exécute les passes dans l'ordre du plan,
/// en écrivant les uniforms de CHAQUE passe dans sa propre soumission
/// (les buffers d'uniforms sont partagés entre passes — le contrat des
/// backends).
#[derive(Debug, Clone, PartialEq)]
pub struct FramePass {
    /// Le label de diagnostic de la passe.
    pub label: String,
    /// La destination de la passe (surface ou cible hors écran).
    pub destination: RenderDestination,
    /// Le traitement de la destination à l'entrée de la passe.
    pub load: PassLoad,
    /// La matrice vue-projection de la passe.
    pub view_projection: Mat4,
    /// La position monde de la caméra de la passe (le spéculaire PBR) —
    /// `Vec3::ZERO` si le consommateur ne l'a pas fournie.
    pub camera_position: Vec3,
    /// Les draws résolus, dans l'ordre de rendu de la RenderQueue.
    pub draws: Vec<FrameDraw>,
    /// Les transforms des instances des draws INSTANCIÉS de la passe —
    /// indexées par les `InstanceRange` des draws, écrites par le
    /// backend dans son instance buffer avant le submit de la passe.
    pub instances: Vec<InstanceTransforms>,
    /// Les batches de DEBUG de la passe — encodés APRÈS `draws`
    /// (opaques → masked → ciel → transparents → debug), l'overlay en
    /// dernier. Vides sans primitives de debug.
    pub debug: Vec<FrameDebugBatch>,
    /// Les sommets LIGNES des batches de debug — indexés par les
    /// `FrameDebugBatch`, écrits par le backend dans son buffer de
    /// debug avant le submit de la passe.
    pub debug_vertices: Vec<DebugVertex>,
}

/// La passe d'OMBRE dérivée du plan — jamais déclarée par le
/// consommateur : le renderer la construit depuis les réglages d'ombre
/// et les casters résolus des passes actives. Le backend l'exécute
/// AVANT toutes les passes, une fois par plan (l'éclairage est global
/// au plan : toutes les passes éclairées échantillonnent la même map).
/// Les draws portent déjà le pipeline d'ombre (depth-only) résolu pour
/// leur vertex layout ; leur `binding` est toujours `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameShadowPass {
    /// La vue-projection de la LUMIÈRE (orthographique alignée sur ses
    /// rayons, cadrée sur le volume d'ombre).
    pub view_projection: Mat4,
    /// La résolution (carrée) de la shadow map visée — le backend doit
    /// posséder une map de cette taille (`set_shadow`).
    pub resolution: u32,
    /// Le biais de profondeur (unités light-clip), appliqué à
    /// l'échantillonnage par les shaders éclairés.
    pub depth_bias: f32,
    /// Le biais de normale (unités monde), appliqué à l'échantillonnage.
    pub normal_bias: f32,
    /// L'index, dans les lumières COLLECTÉES du plan, de la
    /// directionnelle qui projette — le shader n'atténue qu'elle.
    pub light_index: u32,
    /// Les casters résolus (opaques et masked `cast_shadows`, layout à
    /// position) — regroupés en draws instanciés quand ils partagent
    /// leurs buffers.
    pub draws: Vec<FrameDraw>,
    /// Les transforms des instances des casters INSTANCIÉS — le pendant
    /// ombre de `FramePass.instances`.
    pub instances: Vec<InstanceTransforms>,
}

/// L'environnement de la frame, DÉJÀ résolu : l'intensité de la
/// contribution environnementale (0 sans environnement — le cube
/// fallback noir du backend annule de toute façon la contribution) et
/// l'exposition, partagées par toutes les passes du plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameEnvironment {
    /// L'intensité de la contribution environnementale (IBL et ciel).
    pub intensity: f32,
    /// L'exposition appliquée avant le tone mapping (PBR et ciel).
    pub exposure: f32,
}

impl Default for FrameEnvironment {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            exposure: 1.0,
        }
    }
}

/// Description de ce que le renderer doit produire pour une frame : la
/// suite ORDONNÉE des passes à exécuter. L'abstraction décrit le
/// « quoi », le backend exécute le « comment » — il acquiert la surface
/// au plus UNE fois (avant sa première passe surface) et présente au
/// plus UNE fois (après la dernière passe du plan).
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    /// Les passes de la frame, dans l'ordre d'exécution.
    pub passes: Vec<FramePass>,
    /// L'éclairage de la frame, DÉJÀ collecté (filtré, normalisé,
    /// tronqué) — partagé par toutes les passes du plan.
    pub lights: FrameLights,
    /// L'environnement de la frame — partagé par toutes les passes.
    pub environment: FrameEnvironment,
    /// La passe d'ombre dérivée — exécutée AVANT les passes, absente
    /// sans réglages d'ombre ou sans directionnelle qui projette.
    pub shadow: Option<FrameShadowPass>,
}

/// L'issue de la PRÉSENTATION d'une frame : `Rendered` = le travail
/// soumis est parti (et présenté si le plan portait une passe surface) ;
/// `Skipped` = la présentation a été sautée pour une raison nommée — les
/// passes vers des cibles hors écran ont pu s'exécuter quand même.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// Le travail de la frame a été soumis (présenté si passe surface).
    Rendered,
    /// La présentation a été sautée proprement — la raison est nommée.
    Skipped(FrameSkipReason),
}

/// La raison, toujours nommée, d'une frame sautée — jamais un échec
/// silencieux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSkipReason {
    /// La surface n'est pas disponible (perdue, en transition).
    SurfaceUnavailable,
    /// La surface vient d'être reconfigurée — la frame suivante rendra.
    SurfaceReconfigured,
    /// La surface a une aire nulle (fenêtre minimisée) — le rendu est
    /// suspendu jusqu'au retour d'une taille réelle.
    ZeroArea,
}
