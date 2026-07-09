/// Paramètres d'attachement du renderer.
///
/// `vsync` synchronise la présentation sur le rafraîchissement de l'écran ;
/// désactivé, la présentation ne bloque jamais le thread appelant (la cadence
/// est alors régulée par l'hôte, ex. `target_fps` du moteur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererConfig {
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}
