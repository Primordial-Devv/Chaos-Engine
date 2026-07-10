# Tests du moteur

Tout se lance depuis la racine du workspace. Ce document couvre les phases 1 (cycle de vie, fenêtre, événements, boucle) et 2 (renderer minimal), Rendering Core V1, V2 et V3 (pipelines, géométrie, mesh, transforms, caméra, depth, RenderQueue, textures, samplers, bindings, materials, cache), la phase 3 Asset Pipeline (identité, registre, manager, importeurs, cache, validation, hot reload préparé) et la phase 4 ECS Core (entités, composants, world, ressources, systèmes, scheduler, requêtes, messages, commandes, intégration moteur) — et s'étoffera à chaque phase.

## 1. Tests unitaires

```sh
cargo test --workspace
```

| Crate | Tests | Ce qui est vérifié |
|---|---|---|
| `chaos_core` | 28 | Horloge de frame, Color, **Transform** (matrice, TRS, directions locales), **conventions mathématiques verrouillées** (main droite, column-major, rotations, profondeur 0..1), **Camera** (view inverse du transform, projection NDC centrée, composition P×V, aspect au viewport), **AssetId** (identité déterministe, algorithme FNV-1a verrouillé par vecteurs de référence), **Entity** (identité générationnelle, Display stable) |
| `chaos_ecs` | 97 | **Entities** (l'allocateur) : spawn/despawn, recyclage de slot avec génération neuve, entité périmée/forgée rejetée avec erreur explicite, double despawn en erreur, itération des vivantes seulement ; **ComponentStorage** (sparse set) : roundtrip avec Transform réel, remplacement rendant l'ancienne valeur, invariant swap_remove (les autres entités restent résolues), entité périmée jamais résolue, itération dense (Entity, &T), mutation en place ; **World** (le conteneur central) : garantie de vivacité à l'écriture (insert sur mort = erreur explicite nommant le type), despawn détachant tous les composants (index recyclé propre, insert ne déloge rien), despawn périmé sans effet sur l'occupant actuel, types multiples coexistant, valeurs indépendantes entre entités, `World: Send + Sync` verrouillé à la compilation ; **Resources** (les globales sans entité) : roundtrip avec `Time` réel, remplacement rendant l'ancienne valeur, retrait rendant la valeur, coexistence de types indépendants, survie au cycle de vie des entités, registre prouvé étanche aux composants (un même type dans les deux, sans collision) ; **Systems** (les traitements) : exécution dans l'ordre d'enregistrement (trace exacte), système réaliste (Time lu, Transforms avancés du delta), lecture/écriture de ressources, échec arrêtant le tick en nommant le système (mutations antérieures conservées), transformation de la structure du monde (spawn/despawn), doublon de nom rejeté ; **Schedule** (l'ordonnancement) : l'ordre vient des stages et non des appels (enregistrement croisé), ordre intra-stage conservé, même nom légal dans deux stages, doublon intra-stage rejeté, stage inconnu en erreur explicite, échec arrêtant tout en nommant stage + système ; **Requêtes** : 10 000 entités mais seules les correspondances visitées, storage absent → vide, jointure exacte (les deux composants requis), sondage filtrant, mutation du meneur par la sonde, `query2_mut::<T, T>` en erreur explicite, composition des lectures pour jointures larges, corps de système sur requêtes de bout en bout ; **Messages** : FIFO exact, files auto-créées et indépendantes par type, drain consommant, clear balayant toutes les files, `chaos_core::Event` réel en roundtrip, flux complet via le World ; **Commands** : rien ne change avant l'apply, FIFO, despawns enregistrés pendant une requête puis appliqués, échec strict nommant l'index (file consommée, mutations antérieures conservées), patron flush cross-système via la ressource `Commands` ; **isolation** (tests d'intégration) : le renderer et l'Asset Pipeline ne mentionnent jamais chaos_ecs (sources + manifestes), chaos_ecs ne dépend que de chaos_core |
| `chaos_window` | 4 | Traduction winit → types maison : touches, boutons, états, fallback `Unknown` |
| `chaos_assets` | 66 | **AssetRegistry** : enregistrement (id documenté, doublon rejeté en nommant l'existant), lookup nom → id, listage, transitions d'état (loaded/failed/unloaded, id inconnu rejeté) ; **AssetManager** : cycle de vie complet sur fichiers temporaires réels (roundtrip, cache prouvé par suppression du fichier, échec I/O → état Failed consultable, procédural non chargeable, unload idempotent + rechargement) ; **importeurs** : PPM P6 (décodage RGBA exact, commentaires, malformations nommées), WGSL (UTF-8), **glTF** (GLB construit octet par octet en test : positions/UV/indices exacts, UV zéros, séquence d'indices générée, non-TRIANGLES et buffers externes rejetés, octets corrompus nommés ; `.gltf` auto-suffisant via data URI base64, décodeur verrouillé par vecteurs RFC 4648), import de bout en bout, routage kind+extension, importeur custom enregistré, importeur manquant → Failed ; **durée de vie** : acquire/release (mutualisation prouvée), unload protégé par la rétention, évincement des non-retenus, cycle streaming acquire → release → evict → reload ; **porte de validation** : règles sémantiques unitaires (indices hors bornes, NaN, désappariements, dimensions nulles) + importeurs malveillants refusés à la porte avec état Failed ; **hot reload (primitives)** : version de contenu (+1 par matérialisation, insensible aux échecs), reload sous rétention, donnée précédente conservée quand le nouveau fichier est invalide |
| `chaos_renderer` | 87 | Orchestration via backend factice (plan de frame, outcomes, pipelines, shaders, buffers, **textures** : forward du descripteur, validation dimensions/format/pixels portée par le descripteur (`validate()`) et appliquée avant le backend, **samplers** (défauts Linear+Repeat, builders), **bindings** (texture+sampler forward, binding par draw dans le plan), meshes, **uniforms** : view-projection dans le plan, Transform → matrice modèle par draw), géométrie (dont le **cube** : enroulement CCW verrouillé, couleur par face), **RenderQueue** (tri stable par pipeline), **vertex layouts déclaratifs**, **pool générationnel**, + 3 tests d'intégration : 2 d'**isolation wgpu**, 1 **validation naga des `.wgsl` intégrés** |
| `chaos_engine` | 24 | Cycle de vie complet (init/shutdown ordonnés, exits, gating, échecs d'init, update → render) + **contrôleur de caméra debug** (avance selon forward, purge au focus perdu, rotation au drag droit seulement, pas de saut au premier mouvement, pitch clampé, vitesse bornée à la molette) + **couture assets → renderer** (mapping texture/géométrie exacts, garde u16 des gros meshes, appariement UV préservé) + **intégration ECS** (la ressource `Time` alimentée par le tick, un événement pompé = un message pour exactement un update, un système enregistré via `schedule_mut` tourne à chaque frame, un système en échec arrête le moteur proprement) |

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

**Lancer depuis la racine du workspace** (le sol vient de fichiers réels :
`assets/textures/checker.ppm` et `assets/models/floor.glb`, chargés par
l'Asset Pipeline — declare → import → couture → renderer). La fenêtre doit
afficher la **scène pilotée par les materials** — 13 draws
par frame (triplets mesh + material + transform) pour **4 meshes** et **4
materials** : un **sol damier violet** (quad texturé 1×1 étiré en 8×8, posé
à y=-1 — damier 2×2 **neutre blanc/gris** répété ×4 par le sampler
`Nearest`+`Repeat`, teinté par le `base_color` violet du material
`demo.floor`), un **cube central damier ambre** en rotation lente sur deux
axes (`TexturedGeometry::cube` : UV 0..1 sur chaque face — le MÊME damier
que le sol, teinté ambre par le material `demo.cube` : deux materials, une
texture partagée ; **le cube est une entité ECS** : composants `Transform` +
`Spin`, animé par le système `demo.spin` dans `stages::UPDATE` — visuel
inchangé, nouvelle plomberie), une **ronde de 8 cubes 6 couleurs** (mesh coloré partagé,
material `demo.solid` sans texture → fallbacks builtin `chaos.white` +
`chaos.default_sampler`) de tailles (0.3 → 0.72) et vitesses d'orbite/spin
toutes différentes, et **trois triangles dégradés** flottants. Les vitesses
d'orbite différentes font que les cubes **se croisent en permanence** :
l'occlusion doit rester correcte à chaque croisement. La scène traverse
**quatre pipelines** : `demo.geometry` (vertex color, back-face culling),
`demo.geometry.double_sided` (triangles), `demo.floor` (texturé
double-sided) et `demo.textured` (texturé cullé, le cube central). La démo
soumet en ordre de scène ; la **RenderQueue** regroupe par material avant le
backend — visuellement invisible (géométrie opaque + depth buffer), c'est le
point. **Au resize, les proportions sont conservées** (le sol reste carré,
les cubes ne s'étirent pas) : c'est la caméra qui gère l'aspect ratio, plus
l'étirement NDC. Les logs `debug` montrent les quatre pipelines, sept
buffers, quatre meshes, la texture damier, la texture de repli
(`chaos.white`), les deux samplers, les quatre `material binding … created`,
et `object uniform slots grown to 13` — atteint une seule fois à la première
frame, puis les slots sont réutilisés. Le log `renderer released` doit
apparaître au shutdown, avant `engine stopped`.

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
