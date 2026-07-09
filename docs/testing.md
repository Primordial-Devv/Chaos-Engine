# Tests du moteur

Tout se lance depuis la racine du workspace. Ce document couvre les phases 1 (cycle de vie, fenêtre, événements, boucle) et 2 (renderer minimal) ainsi que Rendering Core V1 et V2 (pipelines, géométrie, mesh, transforms, caméra, depth, RenderQueue) et s'étoffera à chaque phase.

## 1. Tests unitaires

```sh
cargo test --workspace
```

| Crate | Tests | Ce qui est vérifié |
|---|---|---|
| `chaos_core` | 20 | Horloge de frame, Color, **Transform** (matrice, TRS, directions locales), **conventions mathématiques verrouillées** (main droite, column-major, rotations, profondeur 0..1), **Camera** (view inverse du transform, projection NDC centrée, composition P×V, aspect au viewport) |
| `chaos_window` | 4 | Traduction winit → types maison : touches, boutons, états, fallback `Unknown` |
| `chaos_renderer` | 46 | Orchestration via backend factice (plan de frame, outcomes, pipelines, shaders, buffers, meshes, **uniforms** : view-projection dans le plan, Transform → matrice modèle par draw), géométrie (dont le **cube** : enroulement CCW verrouillé, couleur par face), **RenderQueue** (tri stable par pipeline), **vertex layouts déclaratifs**, **pool générationnel**, + 3 tests d'intégration : 2 d'**isolation wgpu**, 1 **validation naga des `.wgsl` intégrés** |
| `chaos_engine` | 16 | Cycle de vie complet (init/shutdown ordonnés, exits, gating, échecs d'init, update → render) + **contrôleur de caméra debug** (avance selon forward, purge au focus perdu, rotation au drag droit seulement, pas de saut au premier mouvement, pitch clampé, vitesse bornée à la molette) |

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
INFO  Chaos Sandbox starting (Chaos Engine <version>)
INFO  window ready: <w>x<h> (scale factor <n>)
INFO  graphics adapter selected: wgpu (<GPU> / <Backend>)
INFO  renderer ready: wgpu (<GPU> / <Backend>)
INFO  engine running (2 subsystem(s))
INFO  frame limit reached (180), requesting exit
INFO  engine shutting down
INFO  renderer released
INFO  engine stopped
INFO  Chaos Sandbox stopped cleanly
```

Les 2 subsystems : `geometry_demo` (contenu) + `render_subsystem` (pilote,
enregistré automatiquement en dernier).

Le code de sortie doit être `0` (`echo $?` juste après).

La fenêtre doit afficher la **scène multi-objets** — 13 draws par frame pour
seulement **3 meshes partagés** : un **sol violet sombre** (quad 1×1 étiré en
8×8 par une échelle non uniforme, posé à y=-1), le **cube central 6 couleurs**
en rotation lente sur deux axes, une **ronde de 8 cubes** — le même mesh que
le central — de tailles (0.3 → 0.72) et vitesses d'orbite/spin toutes
différentes, et **trois triangles dégradés** flottants d'échelles distinctes.
Les vitesses d'orbite différentes font que les cubes **se croisent en
permanence** : l'occlusion doit rester correcte à chaque croisement (devant /
derrière le cube central et entre eux). La scène traverse **deux pipelines** :
les cubes fermés passent par `demo.geometry` (back-face culling — corrects
sous tous les angles), sol et triangles par `demo.geometry.double_sided`
(visibles des deux côtés — la sémantique correcte d'une géométrie plate). La
démo soumet en ordre de scène ; la **RenderQueue** regroupe par pipeline avant
le backend — visuellement invisible (géométrie opaque + depth buffer), c'est
le point. **Au resize, les proportions sont conservées** (le sol reste carré,
les cubes ne s'étirent pas) : c'est la caméra qui gère l'aspect ratio, plus
l'étirement NDC. Les logs `debug` montrent les deux pipelines, cinq buffers,
trois meshes, et `object uniform slots grown to 13` — atteint une seule fois
à la première frame, puis les slots sont réutilisés. Le log `renderer
released` doit apparaître au shutdown, avant `engine stopped`.

### Navigation debug dans la scène

La caméra se pilote au clavier/souris (contrôleur `chaos_engine::debug`) :

| Contrôle | Action |
|---|---|
| **Clic droit maintenu + souris** | Regarder (yaw/pitch, pitch clampé ±89°) |
| **Z/Q/S/D** (touches physiques WASD) | Avancer / gauche / reculer / droite |
| **Espace / Shift gauche** | Monter / descendre |
| **Molette** | Vitesse de déplacement (0,1 → 100 m/s) |

Perte de focus (alt-tab) → touches et drag purgés, aucune touche fantôme.

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
