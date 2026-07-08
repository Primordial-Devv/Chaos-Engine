# Tests du moteur

Tout se lance depuis la racine du workspace. Ce document couvre les phases 1 (cycle de vie, fenêtre, événements, boucle) et 2 (renderer minimal) et s'étoffera à chaque phase.

## 1. Tests unitaires

```sh
cargo test --workspace
```

| Crate | Tests | Ce qui est vérifié |
|---|---|---|
| `chaos_core` | 5 | Avance de l'horloge de frame, delta borné à 250 ms, horloge figée → delta zéro, Color (rgb opaque, défaut noir) |
| `chaos_window` | 4 | Traduction winit → types maison : touches, boutons, états, fallback `Unknown` |
| `chaos_engine` | 8 | Init dans l'ordre / shutdown en ordre inverse, `CloseRequested` → exit, dispatch des événements aux subsystems, `frame_limit`, échec d'init (seuls les subsystems initialisés sont arrêtés), update/redraw ignorés avant démarrage, séquence update → render |

Les tests unitaires ne touchent jamais le GPU (la CI n'en a pas) : la validation
GPU est locale, via les runs sandbox ci-dessous.

Cibler une crate et voir le nom de chaque test :

```sh
cargo test -p chaos_engine
```

## 2. Test end-to-end automatisé (sans interaction)

```sh
CHAOS_FRAME_LIMIT=180 cargo run -p sandbox
```

La fenêtre s'ouvre, le moteur tourne 180 frames (~3 s à 60 fps) puis s'arrête seul. Séquence attendue dans les logs :

```
INFO  Chaos Sandbox starting (Chaos Engine 0.1.0)
INFO  window ready: <w>x<h> (scale factor <n>)
INFO  graphics adapter selected: wgpu (<GPU> / <Backend>)
INFO  renderer ready: wgpu (<GPU> / <Backend>)
INFO  engine running (1 subsystem(s))
INFO  frame limit reached (180), requesting exit
INFO  engine shutting down
INFO  engine stopped
INFO  Chaos Sandbox stopped cleanly
```

Le code de sortie doit être `0` (`echo $?` juste après).

La fenêtre doit afficher la couleur de fond du sandbox (violet sombre), rendue
par le GPU. Le log `renderer released` doit apparaître au shutdown, avant
`engine stopped`.

## 3. Test interactif du cycle de vie

```sh
cargo run -p sandbox
```

La fenêtre reste ouverte. Redimensionner, déplacer, changer le focus, puis fermer avec le bouton natif : les logs doivent montrer `close requested by the system` suivi de la séquence d'arrêt propre.

## 4. Trace des événements en temps réel

```sh
RUST_LOG=trace cargo run -p sandbox
```

Chaque événement traduit s'affiche : `CursorMoved` (souris), `MouseButton` (clics), `Keyboard { key, state, repeat }` (clavier), `MouseWheel` (molette), `Resized`, `Moved`, `Focused`. C'est la vérification vivante de la frontière de traduction winit → `chaos_core`.

Variante ciblée, moins bavarde :

```sh
RUST_LOG=info,chaos_engine=trace cargo run -p sandbox
```

## 5. Portes de qualité (identiques à la CI)

```sh
cargo check --workspace --all-targets
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Si ces trois commandes et les tests passent en local, la CI de la PR sera verte.

## Leviers de test du moteur

| Levier | Où | Effet |
|---|---|---|
| `CHAOS_FRAME_LIMIT=<n>` | env, lu par `sandbox` | Renseigne `EngineConfig::frame_limit` : arrêt propre après n frames |
| `EngineConfig::frame_limit` | code | Même effet, pour tout hôte du moteur |
| `EngineConfig::target_fps` | code | `None` = boucle libre (utile en test pour éviter le pacing), `Some(n)` = cadence via l'attente native de l'OS |
| `EngineConfig::vsync` | code | `false` par défaut (présentation non bloquante — évite le lag d'interactions macOS), `true` = synchronisation écran |
| `RUST_LOG` | env (`env_logger` dans sandbox) | Niveau de logs : `error`/`warn`/`info`/`debug`/`trace`, filtrable par module |
