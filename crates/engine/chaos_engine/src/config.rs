use std::time::Duration;

use chaos_core::{ChaosError, ChaosResult, Color, FixedClock};
use chaos_window::WindowConfig;

/// L'APPLICATION lancée : son identité aux yeux du moteur — les logs de
/// démarrage et d'arrêt aujourd'hui, le nommage des builds et des données
/// du runtime demain.
#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    /// Le nom de l'application — jamais vide (validé).
    pub name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: String::from("Chaos Engine"),
        }
    }
}

/// Le RENDU : la politique de présentation portée par le moteur et
/// appliquée au renderer à son attachement.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderConfig {
    /// La synchronisation verticale de la présentation — désactivée par
    /// défaut : un present bloquant sur le main thread rend les
    /// interactions fenêtre (déplacement, resize) laggy sur macOS.
    pub vsync: bool,
    /// La couleur de fond présentée par le renderer — composantes finies
    /// (validé).
    pub clear_color: Color,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            vsync: false,
            clear_color: Color::rgb(0.02, 0.02, 0.03),
        }
    }
}

/// Le TEMPS : la cadence de la boucle et le pas de la simulation fixe.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeConfig {
    /// La cadence cible de la boucle (via l'attente native de l'OS,
    /// jamais un sleep bloquant) ; `None` laisse la boucle libre — jamais
    /// `Some(0)` (validé).
    pub target_fps: Option<u32>,
    /// Le pas de la simulation FIXE (`stages::FIXED_UPDATE`) — 1/60 s par
    /// défaut, strictement positif (validé) ; le rattrapage par frame est
    /// borné par le moteur (anti-spirale de la mort).
    pub fixed_timestep: Duration,
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            target_fps: Some(60),
            fixed_timestep: FixedClock::DEFAULT_STEP,
        }
    }
}

/// Les LOGS : la politique de filtrage portée par la configuration —
/// APPLIQUÉE PAR L'APPLICATION : l'initialisation du logger appartient au
/// binaire, jamais au moteur (le sandbox montre le patron).
#[derive(Debug, Clone, PartialEq)]
pub struct LogConfig {
    /// Le filtre par défaut du logger (syntaxe `env_logger` : `"info"`,
    /// `"chaos_renderer=debug"`, …) — `RUST_LOG` garde toujours la
    /// priorité ; `None` laisse le logger sur son propre défaut ; jamais
    /// vide (validé).
    pub filter: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: Some(String::from("info")),
        }
    }
}

/// L'EXÉCUTION : le mode de fonctionnement global du moteur — les leviers
/// des tests et de la CI aujourd'hui, du runtime et du serveur dédié
/// demain.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeConfig {
    /// Le mode sans fenêtre ni renderer (serveur dédié, tests, outils,
    /// CI) : la boucle logique COMPLÈTE tourne — ECS, scènes, assets,
    /// temps, subsystems — mais la phase présentation n'existe pas
    /// (`Subsystem::render` jamais appelé, `renderer()` reste `None`).
    /// Les subsystems déclarant `requires_graphics()` sont retirés au
    /// démarrage ; la pause y est indisponible (aucun canal d'événements
    /// pour la lever).
    pub headless: bool,
    /// L'arrêt propre après N frames (tests, CI, soak) — jamais `Some(0)`
    /// (validé).
    pub frame_limit: Option<u64>,
    /// Pause AUTOMATIQUE à la perte de focus (reprise au retour) —
    /// désactivée par défaut : l'application choisit. Une pause demandée
    /// par l'app n'est jamais reprise par un retour de focus.
    pub pause_on_focus_loss: bool,
    /// Les subsystems désactivés par leur nom : retirés AVANT le tri des
    /// dépendances — jamais initialisés, jamais tickés. Un nom inconnu au
    /// démarrage est refusé ; un doublon ou un nom vide aussi (validé).
    pub disabled_subsystems: Vec<String>,
}

/// La configuration de démarrage du moteur — LE modèle que l'éditeur
/// (Project Settings), le runtime, le serveur dédié et les builds
/// consommeront ; chaque domaine est un groupe distinct.
///
/// Le cycle complet : des défauts sûrs (`Default`) → la surcharge par
/// l'application (littéral + `..Default::default()`) → `validate()` par le
/// moteur AVANT toute initialisation partielle → la consultation contrôlée
/// (le moteur lit, jamais ne réécrit). Une configuration invalide échoue
/// avec une erreur précise (`ChaosError::Config`), jamais un rattrapage
/// silencieux.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EngineConfig {
    pub app: AppConfig,
    pub window: WindowConfig,
    pub render: RenderConfig,
    pub time: TimeConfig,
    pub logs: LogConfig,
    pub runtime: RuntimeConfig,
}

impl EngineConfig {
    /// Valide la configuration ENTIÈRE — appelée par le moteur avant toute
    /// initialisation partielle ; la première règle violée rend une erreur
    /// précise et exploitable.
    pub fn validate(&self) -> ChaosResult<()> {
        if self.app.name.trim().is_empty() {
            return Err(ChaosError::Config(String::from(
                "app.name must not be empty",
            )));
        }
        if self.window.width == 0 || self.window.height == 0 {
            return Err(ChaosError::Config(format!(
                "window size {}x{} is invalid: both dimensions must be at least 1",
                self.window.width, self.window.height
            )));
        }
        let color = self.render.clear_color;
        if [color.r, color.g, color.b, color.a]
            .iter()
            .any(|component| !component.is_finite())
        {
            return Err(ChaosError::Config(format!(
                "render.clear_color has a non-finite component (r={} g={} b={} a={})",
                color.r, color.g, color.b, color.a
            )));
        }
        if self.time.target_fps == Some(0) {
            return Err(ChaosError::Config(String::from(
                "time.target_fps must be at least 1: use None for an unpaced loop",
            )));
        }
        if self.time.fixed_timestep.is_zero() {
            return Err(ChaosError::Config(String::from(
                "time.fixed_timestep must be strictly positive",
            )));
        }
        if self
            .logs
            .filter
            .as_deref()
            .is_some_and(|filter| filter.trim().is_empty())
        {
            return Err(ChaosError::Config(String::from(
                "logs.filter must not be empty: use None to keep the logger default",
            )));
        }
        if self.runtime.frame_limit == Some(0) {
            return Err(ChaosError::Config(String::from(
                "runtime.frame_limit must be at least 1: use None to run unlimited",
            )));
        }
        for (index, name) in self.runtime.disabled_subsystems.iter().enumerate() {
            if name.trim().is_empty() {
                return Err(ChaosError::Config(String::from(
                    "runtime.disabled_subsystems contains an empty name",
                )));
            }
            if self.runtime.disabled_subsystems[..index].contains(name) {
                return Err(ChaosError::Config(format!(
                    "runtime.disabled_subsystems lists '{name}' twice"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(config: EngineConfig, expected: &str) {
        let error = config.validate().unwrap_err().to_string();
        assert!(error.starts_with("configuration error: "), "{error}");
        assert!(error.contains(expected), "{error}");
    }

    #[test]
    fn the_default_configuration_is_valid() {
        assert_eq!(EngineConfig::default().validate(), Ok(()));
    }

    #[test]
    fn an_application_override_stays_valid() {
        let config = EngineConfig {
            app: AppConfig {
                name: String::from("Chaos Sandbox"),
            },
            window: WindowConfig {
                title: String::from("Chaos Sandbox"),
                ..WindowConfig::default()
            },
            render: RenderConfig {
                clear_color: Color::rgb(0.10, 0.03, 0.18),
                ..RenderConfig::default()
            },
            runtime: RuntimeConfig {
                frame_limit: Some(240),
                ..RuntimeConfig::default()
            },
            ..EngineConfig::default()
        };
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn an_empty_app_name_is_refused() {
        refused(
            EngineConfig {
                app: AppConfig {
                    name: String::from("   "),
                },
                ..EngineConfig::default()
            },
            "app.name",
        );
    }

    #[test]
    fn a_degenerate_window_size_is_refused() {
        refused(
            EngineConfig {
                window: WindowConfig {
                    width: 0,
                    ..WindowConfig::default()
                },
                ..EngineConfig::default()
            },
            "window size 0x",
        );
        refused(
            EngineConfig {
                window: WindowConfig {
                    height: 0,
                    ..WindowConfig::default()
                },
                ..EngineConfig::default()
            },
            "at least 1",
        );
    }

    #[test]
    fn a_non_finite_clear_color_is_refused() {
        refused(
            EngineConfig {
                render: RenderConfig {
                    clear_color: Color::rgb(f32::NAN, 0.0, 0.0),
                    ..RenderConfig::default()
                },
                ..EngineConfig::default()
            },
            "clear_color",
        );
    }

    #[test]
    fn a_degenerate_time_setting_is_refused() {
        refused(
            EngineConfig {
                time: TimeConfig {
                    target_fps: Some(0),
                    ..TimeConfig::default()
                },
                ..EngineConfig::default()
            },
            "target_fps",
        );
        refused(
            EngineConfig {
                time: TimeConfig {
                    fixed_timestep: Duration::ZERO,
                    ..TimeConfig::default()
                },
                ..EngineConfig::default()
            },
            "fixed_timestep",
        );
    }

    #[test]
    fn an_empty_log_filter_is_refused() {
        refused(
            EngineConfig {
                logs: LogConfig {
                    filter: Some(String::new()),
                },
                ..EngineConfig::default()
            },
            "logs.filter",
        );
    }

    #[test]
    fn a_headless_configuration_is_valid() {
        let config = EngineConfig {
            runtime: RuntimeConfig {
                headless: true,
                frame_limit: Some(240),
                ..RuntimeConfig::default()
            },
            ..EngineConfig::default()
        };
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn a_zero_frame_limit_is_refused() {
        refused(
            EngineConfig {
                runtime: RuntimeConfig {
                    frame_limit: Some(0),
                    ..RuntimeConfig::default()
                },
                ..EngineConfig::default()
            },
            "frame_limit",
        );
    }

    #[test]
    fn a_degenerate_disabled_subsystem_list_is_refused() {
        refused(
            EngineConfig {
                runtime: RuntimeConfig {
                    disabled_subsystems: vec![String::from("audio"), String::from("audio")],
                    ..RuntimeConfig::default()
                },
                ..EngineConfig::default()
            },
            "'audio' twice",
        );
        refused(
            EngineConfig {
                runtime: RuntimeConfig {
                    disabled_subsystems: vec![String::from(" ")],
                    ..RuntimeConfig::default()
                },
                ..EngineConfig::default()
            },
            "empty name",
        );
    }
}
