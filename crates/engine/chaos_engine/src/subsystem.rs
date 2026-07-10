use chaos_core::{ChaosResult, Event};

use crate::context::EngineContext;

/// Point d'accroche des systèmes du moteur (renderer, scènes, ECS, physique,
/// audio, réseau, runtime…).
///
/// **Le cycle de vie** : Enregistré (`add_subsystem`) → trié par
/// dépendances → initialisé → **Actif** (reçoit `on_event`/`update`/
/// `render`, dans l'ordre trié) → Arrêté (`shutdown`, en ordre INVERSE) →
/// Détruit (drop). Un échec d'init n'arrête que les subsystems déjà
/// initialisés, en ordre inverse.
///
/// **Les dépendances se déclarent par NOM** — couplage faible : des
/// chaînes, jamais des types. Le moteur trie topologiquement au démarrage
/// (à égalité, l'ordre d'enregistrement départage — déterminisme total) ;
/// cycle, dépendance absente ou nom dupliqué = démarrage refusé avec
/// erreur explicite.
///
/// **La classification** — le moteur SAIT ce qu'est chaque subsystem :
/// **graphique** (`requires_graphics()` : exige fenêtre + renderer —
/// retiré au démarrage en mode headless), **compatible headless** (le
/// défaut), **optionnel** (désactivable par
/// `runtime.disabled_subsystems`), **obligatoire** (encodé par les
/// DÉPENDANCES : retirer un subsystem dont un autre dépend refuse le
/// démarrage). La garde interne (`context.renderer()` absent → no-op)
/// reste une défense en profondeur, mais la déclaration explicite est le
/// mécanisme premier ; une API d'activation individuelle viendra avec son
/// besoin réel (l'éditeur).
///
/// **Citoyen du MAIN THREAD, par politique** : un subsystem reçoit
/// `&mut EngineContext` en séquentiel — c'est l'unité d'ORCHESTRATION,
/// jamais de parallélisme. Le parallélisme viendra de l'INTÉRIEUR
/// (systèmes ECS `Send + Sync`, futurs jobs), pas du déplacement des
/// subsystems ; d'où l'absence délibérée de borne `Send` ici (la
/// politique complète : `docs/architecture/threading.md`).
pub trait Subsystem {
    fn name(&self) -> &str;

    /// Les NOMS des subsystems qui doivent être initialisés AVANT
    /// celui-ci (et arrêtés après). Vide par défaut.
    fn dependencies(&self) -> &[&str] {
        &[]
    }

    /// Ce subsystem exige-t-il la pile graphique (fenêtre + renderer) ?
    /// `false` par défaut (compatible headless). Déclarer `true` le fait
    /// RETIRER au démarrage en mode headless — jamais initialisé, jamais
    /// tické, avec `info!` ; un subsystem restant qui en dépend refuse le
    /// démarrage (dépendance manquante, nommée).
    fn requires_graphics(&self) -> bool {
        false
    }

    fn init(&mut self, _context: &mut EngineContext) -> ChaosResult<()> {
        Ok(())
    }

    fn on_event(&mut self, _event: &Event, _context: &mut EngineContext) {}

    fn update(&mut self, _context: &mut EngineContext) {}

    fn render(&mut self, _context: &mut EngineContext) {}

    fn shutdown(&mut self, _context: &mut EngineContext) {}
}
