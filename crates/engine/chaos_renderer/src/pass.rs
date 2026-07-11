use chaos_core::math::{Mat4, Vec3};
use chaos_core::{ChaosError, ChaosResult, Color};

use crate::frame::RenderDestination;
use crate::resources::RenderTargetHandle;

/// Identifiant opaque d'une passe déclarée. Les passes sont PERMANENTES
/// en V1 : une passe ne se supprime pas, elle se DÉSACTIVE
/// (`Renderer::set_pass_enabled`) — le handle reste simple, jamais
/// générationnel (la symétrie des pipelines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassHandle(pub(crate) u32);

/// Le traitement de la destination à l'ENTRÉE d'une passe : effacée à la
/// couleur donnée, ou conservée. `Keep` porte sur la COULEUR — la
/// profondeur suit ses propres règles côté backend (chargée seulement si
/// une passe antérieure de la même destination l'a produite cette frame,
/// effacée sinon : jamais de contenu indéfini).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PassLoad {
    /// Efface la destination à cette couleur au début de la passe.
    Clear(Color),
    /// Conserve la couleur existante — la passe dessine par-dessus.
    Keep,
}

/// Description d'une passe de rendu déclarée — l'unité d'orchestration de
/// la frame. Une passe possède sa destination, son traitement d'entrée,
/// sa caméra, ses LECTURES déclarées (les cibles que ses materials
/// échantillonnent — la matière de la validation d'ordonnancement) et son
/// ordre d'exécution.
///
/// L'ordre est DÉTERMINISTE : tri stable par `order` croissant, les
/// égalités départagées par l'ordre d'enregistrement — jamais un tri
/// topologique (le render graph n'est pas l'objet de cette V1). La passe
/// principale `chaos.main` du Renderer a l'ordre 0 : un `order` négatif
/// s'exécute avant elle, un positif après.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPassDescriptor {
    /// Le label de diagnostic ET l'identité déclarée de la passe (unique ;
    /// le préfixe `chaos.` est réservé aux passes du moteur).
    pub label: String,
    /// La destination de la passe (surface ou cible hors écran).
    pub destination: RenderDestination,
    /// Le traitement de la destination à l'entrée de la passe.
    pub load: PassLoad,
    /// La matrice vue-projection de la passe (sa caméra).
    pub view_projection: Mat4,
    /// La position monde de la caméra — consommée par le spéculaire PBR.
    /// `Vec3::ZERO` par défaut (les modèles non-PBR ne la lisent pas).
    pub camera_position: Vec3,
    /// Les cibles que la passe LIT (échantillonne via ses materials) —
    /// déclaratif : si une passe de la même frame écrit une cible lue,
    /// elle doit être ordonnée AVANT la lectrice, sinon la déclaration
    /// est refusée. Une lecture sans écrivain la même frame est légale
    /// (contenu d'une frame précédente).
    pub reads: Vec<RenderTargetHandle>,
    /// L'ordre d'exécution (0 = la passe principale ; négatif = avant,
    /// positif = après ; égalité départagée par l'enregistrement).
    pub order: i32,
    /// Une passe désactivée est sautée proprement à chaque frame — ses
    /// draws sont acceptés puis vidés, rien n'est rendu.
    pub enabled: bool,
}

impl RenderPassDescriptor {
    /// Descripteur aux défauts standard : efface au noir, caméra
    /// identité, aucune lecture, ordre 0, activée.
    pub fn new(label: impl Into<String>, destination: RenderDestination) -> Self {
        Self {
            label: label.into(),
            destination,
            load: PassLoad::Clear(Color::BLACK),
            view_projection: Mat4::IDENTITY,
            camera_position: Vec3::ZERO,
            reads: Vec::new(),
            order: 0,
            enabled: true,
        }
    }

    /// Remplace le traitement d'entrée de la destination.
    pub fn with_load(mut self, load: PassLoad) -> Self {
        self.load = load;
        self
    }

    /// Remplace la caméra de la passe.
    pub fn with_camera(mut self, view_projection: Mat4) -> Self {
        self.view_projection = view_projection;
        self
    }

    /// Fixe la position monde de la caméra (le spéculaire PBR).
    pub fn with_camera_position(mut self, camera_position: Vec3) -> Self {
        self.camera_position = camera_position;
        self
    }

    /// Déclare les cibles que la passe lit.
    pub fn with_reads(mut self, reads: &[RenderTargetHandle]) -> Self {
        self.reads = reads.to_vec();
        self
    }

    /// Remplace l'ordre d'exécution.
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Vérifie la cohérence PROPRE du descripteur (label non vide, pas de
    /// boucle de feedback déclarée) — les règles de registre (unicité,
    /// cibles vivantes, ordonnancement) appartiennent au Renderer.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.label.is_empty() {
            return Err(ChaosError::Graphics(String::from(
                "render pass label cannot be empty",
            )));
        }
        if let RenderDestination::Target(target) = self.destination
            && self.reads.contains(&target)
        {
            return Err(ChaosError::Graphics(format!(
                "render pass '{}' reads its own destination: a feedback loop",
                self.label
            )));
        }
        Ok(())
    }
}

/// L'issue d'une passe de la dernière frame orchestrée — le diagnostic
/// lisible de l'orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOutcome {
    /// La passe a été exécutée par le backend.
    Executed,
    /// La passe était désactivée — sautée proprement.
    Disabled,
    /// La destination de la passe est périmée (cible détruite ou
    /// redimensionnée) — la passe s'est AUTO-DÉSACTIVÉE avec un warn
    /// unique ; `update_pass` avec le handle frais la réactive.
    StaleTarget,
    /// La surface était indisponible (minimisée, en transition) — les
    /// passes surface de la frame ont été sautées, les passes cible ont
    /// pu s'exécuter.
    SurfaceSkipped,
}

/// La ventilation des draws d'une passe par catégorie d'opacité — le
/// diagnostic qui rend l'ordre à quatre temps OBSERVABLE (opaques →
/// masked → ciel → transparents triés).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrawBreakdown {
    /// Les draws OPAQUES résolus.
    pub opaque: usize,
    /// Les draws MASKED (alpha cutout) résolus.
    pub masked: usize,
    /// Les draws TRANSPARENTS résolus (triés arrière → avant).
    pub transparent: usize,
    /// Les draws INJECTÉS par le renderer (le ciel, et chaque primitive
    /// de debug DESSINÉE) — jamais soumis par le consommateur.
    pub injected: usize,
}

/// Le rapport d'une passe de la dernière frame orchestrée.
#[derive(Debug, Clone, PartialEq)]
pub struct PassReport {
    /// Le label de la passe.
    pub label: String,
    /// Sa destination.
    pub destination: RenderDestination,
    /// Le nombre d'OBJETS logiques résolus de la passe (un draw
    /// instancié en compte autant que d'instances) — la sémantique
    /// historique.
    pub draws: usize,
    /// Les soumissions GPU réelles de la passe — l'instancing les rend
    /// ≤ `draws` (le diagnostic du bénéfice).
    pub draw_calls: usize,
    /// Les objets REJETÉS par le frustum de LA passe (hors champ) —
    /// jamais résolus, jamais soumis : le diagnostic du culling.
    pub culled: usize,
    /// La ventilation des draws par catégorie d'opacité.
    pub breakdown: DrawBreakdown,
    /// Son issue.
    pub outcome: PassOutcome,
}

/// Le rapport de la passe d'OMBRE dérivée du plan — un rapport DÉDIÉ,
/// jamais une `PassReport` : sa destination est la shadow map interne
/// du backend, pas une `RenderDestination` déclarable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowReport {
    /// Les casters résolus soumis à la passe d'ombre (les OBJETS
    /// logiques — un draw instancié en compte autant que d'instances).
    pub draws: usize,
    /// Les soumissions GPU réelles de la passe d'ombre — l'instancing
    /// les rend ≤ `draws`.
    pub draw_calls: usize,
    /// Les tentatives de moisson REJETÉES par le frustum de la LUMIÈRE
    /// (hors du volume d'ombre) — les duplicatas multi-passes comptent
    /// comme les draws (documenté).
    pub culled: usize,
    /// La résolution (carrée) de la shadow map visée.
    pub resolution: u32,
}

/// Le rapport de la dernière frame orchestrée (`Renderer::frame_report`),
/// passe par passe dans l'ordre d'exécution. C'est une RECONSTRUCTION du
/// renderer (le backend rend une seule issue de présentation) — vide
/// avant la première frame, valide jusqu'au prochain `render_frame` ;
/// `render_to_target` n'y touche pas.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameReport {
    /// Les passes de la frame, dans l'ordre d'exécution.
    pub passes: Vec<PassReport>,
    /// Le rapport de la passe d'ombre — `None` sans réglages d'ombre ou
    /// sans directionnelle qui projette cette frame.
    pub shadow: Option<ShadowReport>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::RenderTargetHandle;

    #[test]
    fn the_descriptor_validates_itself() {
        let empty = RenderPassDescriptor::new("", RenderDestination::Surface);
        assert!(
            empty
                .validate()
                .unwrap_err()
                .to_string()
                .contains("cannot be empty")
        );

        let target = RenderTargetHandle {
            index: 0,
            generation: 0,
        };
        let feedback = RenderPassDescriptor::new("loop", RenderDestination::Target(target))
            .with_reads(&[target]);
        assert!(
            feedback
                .validate()
                .unwrap_err()
                .to_string()
                .contains("feedback loop")
        );

        let sane = RenderPassDescriptor::new("scene", RenderDestination::Surface)
            .with_load(PassLoad::Keep)
            .with_order(-5);
        assert_eq!(sane.validate(), Ok(()));
        assert_eq!(sane.order, -5);
        assert_eq!(sane.load, PassLoad::Keep);
    }
}
