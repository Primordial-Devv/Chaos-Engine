use chaos_core::SceneId;
use chaos_ecs::Component;

/// L'appartenance d'une entité à une scène — un COMPOSANT ECS, jamais une
/// liste parallèle dans la scène. La relation vit dans le World, la source
/// de vérité unique : le despawn détache tous les composants, donc une
/// entité morte disparaît de sa scène automatiquement — la référence
/// périmée est impossible par construction. Une entité SANS `SceneMember`
/// est globale/persistante : la distinction est structurelle.
///
/// Public : les closures de `Commands` (et plus tard le réseau) insèrent
/// `SceneMember::new(id)` comme n'importe quel composant — `SceneId` est
/// Copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneMember {
    scene: SceneId,
}

impl SceneMember {
    pub fn new(scene: SceneId) -> Self {
        Self { scene }
    }

    pub fn scene(&self) -> SceneId {
        self.scene
    }
}

impl Component for SceneMember {}
