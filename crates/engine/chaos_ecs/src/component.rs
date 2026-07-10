use chaos_core::{GlobalTransform, Transform};

/// Un composant : des DONNÉES attachées à une entité — jamais de
/// comportement (le comportement appartiendra aux systèmes). L'opt-in est
/// explicite : implémenter ce marqueur documente l'intention. La contrainte
/// `Send + Sync + 'static` prépare le parallélisme futur par contrainte
/// (gratuite aujourd'hui), pas par machinerie.
pub trait Component: Send + Sync + 'static {}

impl Component for Transform {}

impl Component for GlobalTransform {}
