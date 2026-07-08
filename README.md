# Chaos Engine

Moteur de jeu privé et plateforme sandbox moddable, dans l'esprit de GMod / FiveM / Nanos World. L'engine est une fondation technique privée et scellée ; la moddabilité (serveurs, gamemodes, scripts, contenus) vit dans la couche plateforme construite au-dessus, via une API contrôlée.

## Architecture en couches

Flux de dépendances strictement descendant : `apps → platform → engine → foundation`. Détails dans `docs/architecture/overview.md`.

| Couche | Contenu | Visibilité |
|---|---|---|
| `crates/foundation/` | `chaos_core` — types partagés, erreurs, utilitaires | Interne |
| `crates/engine/` | `chaos_engine` (façade), `chaos_window`, `chaos_renderer`, `chaos_ecs`, `chaos_scene`, `chaos_assets`, `chaos_physics`, `chaos_audio`, `chaos_network` | **Privée** |
| `crates/platform/` | `chaos_api` (contrat public), `chaos_scripting` (hôte de script sandboxé), `chaos_runtime` (le pont plateforme ↔ engine) | `chaos_api` = surface moddable |
| `crates/tools/` | `chaos_tools` — outillage interne | Interne |

## Exécutables

| App | Rôle | Dépend de |
|---|---|---|
| `apps/sandbox` | Client de test de la plateforme | `chaos_runtime` |
| `apps/dedicated_server` | Serveur dédié de la plateforme | `chaos_runtime` |
| `apps/editor` | Éditeur du moteur | `chaos_engine` |

## Autres dossiers

| Dossier | Rôle |
|---|---|
| `assets/` | Ressources globales (modèles, textures, shaders, audio…) |
| `mods/` | Contenus de la plateforme — ciblent `chaos_api`, jamais l'engine |
| `tools/` | Outils externes (pipeline d'assets, packaging, launcher…) |
| `docs/` | Documentation technique interne |
| `examples/` | Futurs exemples d'utilisation |
| `tests/` | Tests globaux (intégration, assets, réseau, modding) |

## Versioning

Version technique `MAJOR.MINOR.PATCH(+BUILD)` (SemVer), nom public `Chaos N` où `N` = MAJOR. Source de vérité unique : `[workspace.package].version` dans le `Cargo.toml` racine. Convention complète dans `docs/versioning.md`.

## Commandes

```sh
cargo check --workspace
cargo fmt --all
cargo clippy --workspace --all-targets
```

## Statut

Phase 1 terminée : le moteur démarre, ouvre une fenêtre native (Windows/macOS via winit), reçoit les événements système et les entrées clavier/souris, exécute une boucle stable avec horloge de frame bornée, et s'arrête proprement (subsystems arrêtés en ordre inverse). Architecture de la boucle : `docs/architecture/engine-loop.md`.

```sh
cargo run -p sandbox
CHAOS_FRAME_LIMIT=180 cargo run -p sandbox
```

Prochaines phases : renderer, ECS, scènes, assets, physique, audio, réseau, runtime/plateforme.
