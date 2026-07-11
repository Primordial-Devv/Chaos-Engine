use chaos_core::AssetId;
use chaos_ecs::Component;

/// La référence de mesh d'une entité de scène — une IDENTITÉ stable
/// (`AssetId`), jamais un handle GPU : la scène décrit, le renderer
/// consomme, l'app (puis l'extraction de rendu) résout l'identité vers ses
/// handles. Les meshes procéduraux reçoivent la leur via
/// `AssetSource::Procedural` : l'Asset Pipeline est l'espace de noms de
/// résolution, même pour le contenu généré. Le premier composant porteur
/// de références d'assets — l'association material reste une politique
/// d'app (le format de material est un futur documenté).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshRef {
    mesh: AssetId,
}

impl MeshRef {
    pub fn new(mesh: AssetId) -> Self {
        Self { mesh }
    }

    pub fn mesh(&self) -> AssetId {
        self.mesh
    }
}

impl Component for MeshRef {}
