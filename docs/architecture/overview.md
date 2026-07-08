# Architecture de Chaos Engine

## Engine moddable vs plateforme moddable

Chaos Engine n'est **pas** un engine moddable : personne n'étend ni ne modifie ses internes (renderer, ECS, physique…). C'est une fondation privée, libre d'évoluer sans casser qui que ce soit.

La moddabilité est portée par la **plateforme** construite au-dessus, sur le modèle GMod / FiveM / Nanos World : les créateurs développent des serveurs, gamemodes, scripts, maps et assets contre une API contrôlée et versionnée (`chaos_api`), jamais contre l'engine. L'engine peut être entièrement refactoré tant que le contrat de `chaos_api` est respecté.

## Les quatre couches

Flux de dépendances strictement descendant : `apps → platform → engine → foundation`.

| Couche | Crates | Rôle | Visibilité |
|---|---|---|---|
| `crates/foundation/` | `chaos_core` | Types partagés, erreurs, utilitaires | Interne |
| `crates/engine/` | `chaos_engine` (façade), `chaos_window`, `chaos_renderer`, `chaos_ecs`, `chaos_scene`, `chaos_assets`, `chaos_physics`, `chaos_audio`, `chaos_network` | Fondation technique du moteur | **Privée** — jamais exposée aux mods |
| `crates/platform/` | `chaos_api`, `chaos_scripting`, `chaos_runtime` | Plateforme sandbox moddable | `chaos_api` = surface publique |
| `crates/tools/` | `chaos_tools` | Outillage interne | Interne |

## Graphe de dépendances

| Crate | Dépend de |
|---|---|
| `chaos_core` | — |
| `chaos_window`, `chaos_renderer`, `chaos_ecs`, `chaos_assets`, `chaos_physics`, `chaos_audio`, `chaos_network` | `chaos_core` |
| `chaos_scene` | `chaos_core`, `chaos_ecs` |
| `chaos_engine` | `chaos_core` + tous les sous-systèmes engine |
| `chaos_api` | `chaos_core` uniquement |
| `chaos_scripting` | `chaos_core`, `chaos_api` |
| `chaos_runtime` | `chaos_core`, `chaos_engine`, `chaos_api`, `chaos_scripting` |
| `chaos_tools` | `chaos_core` |
| `sandbox`, `dedicated_server` | `chaos_runtime` |
| `editor` | `chaos_engine` |

### Règles

1. **Aucune dépendance montante** : une crate `engine/` ne dépend jamais de `platform/` ; `foundation/` ne dépend de rien.
2. **Pas de dépendance latérale** entre sous-systèmes engine, sauf `chaos_scene → chaos_ecs` (les scènes sont bâties sur l'ECS). Toute nouvelle exception doit être justifiée ici.
3. **`chaos_api` et `chaos_scripting` ne connaissent pas l'engine.** Le contrat public et l'hôte de script restent stables quel que soit le refactor interne du moteur.
4. **`chaos_runtime` est le seul pont** entre la plateforme et l'engine. C'est lui qui implémente `chaos_api` au-dessus de `chaos_engine`.
5. Ces arêtes sont câblées dans les `Cargo.toml` : `cargo check` valide le graphe en permanence.

## Rôle des crates plateforme

- **`chaos_api`** — le contrat : types, événements et interfaces visibles par les gamemodes, scripts et serveurs. Stable, versionné, documenté. C'est la seule surface que voient les créateurs de contenu.
- **`chaos_scripting`** — l'hôte : VM de script sandboxée et bindings exposant `chaos_api` aux langages de script. Ce n'est pas une API de modding de l'engine.
- **`chaos_runtime`** — la plateforme : implémente `chaos_api` au-dessus de l'engine, héberge le scripting, gère serveurs, sessions, gamemodes et chargement de contenu.

## Évolutions prévues (rien de codé)

- Choix de la VM de script (Lua / WASM / JS) dans `chaos_scripting`.
- Crate de protocole client/serveur partagée quand le réseau prendra forme.
- Versionnement sémantique de `chaos_api` dès la première API réelle.
