# Renderer — architecture

Référence des choix de la phase 2 (renderer minimal). Le principe directeur : **wgpu est un détail d'implémentation, jamais une dépendance du moteur.**

## Les deux étages

```
chaos_engine ──► Renderer (API orientée moteur, vocabulaire chaos_core)
                   └─► trait GraphicsBackend (le point de remplacement)
                         └─► WgpuBackend (seule zone du moteur qui importe wgpu)
```

- **`Renderer`** (`renderer.rs`) — ce que voit le moteur : `attach(target, RendererConfig)`, `resize`, `set_clear_color`, `render_frame`, `description`. Ne parle que le vocabulaire de `chaos_core` (`Color`, `ChaosResult`). Testable sans GPU par injection de backend (`with_backend`, interne) — les tests unitaires de la crate vérifient l'orchestration avec un backend factice.
- **`GraphicsBackend`** (`backend/mod.rs`) — le contrat qu'un backend doit honorer. Remplacer wgpu par un backend maison (Vulkan, DirectX 12, Metal) = implémenter ce trait, rien d'autre à toucher dans le moteur.
- **`WgpuBackend`** (`backend/wgpu/`) — l'unique zone du workspace qui importe wgpu. Détient surface, device, queue et configuration.

## Carte des modules

```
chaos_renderer/src/
├── lib.rs           façade publique : Renderer, RenderQueue, Geometry, MeshHandle, descripteurs…
├── config.rs        RendererConfig — paramètres d'attachement (dimensions, vsync)
├── renderer.rs      Renderer — orchestrateur haut niveau + registre des meshes (+ tests mock)
├── frame.rs         DrawCommand / FrameDraw / FramePlan / FrameOutcome / FrameSkipReason
├── queue.rs         RenderQueue — ordre de rendu (tri stable par pipeline)
├── geometry.rs      Geometry — données CPU + constructeurs triangle / quad / cube
├── mesh.rs          MeshHandle + MeshRecord (le mesh possède ses buffers)
├── pool.rs          ResourcePool générationnel (privé) — détection des handles périmés
├── shaders.rs       ShaderLibrary + noms builtin (chaos.*)
├── target.rs        SurfaceTarget — couture raw-window-handle avec la fenêtre
├── resources/       vocabulaire des ressources, indépendant du backend
│   ├── buffer.rs        BufferDescriptor / BufferHandle / BufferKind + bytes_of_*
│   ├── pipeline.rs      PipelineDescriptor / PipelineHandle / topology / cull / front face
│   ├── shader.rs        ShaderRef (Named | Inline) / ShaderSource (Wgsl)
│   └── vertex.rs        VertexLayout / VertexAttribute / ColorVertex
└── backend/
    ├── mod.rs       trait GraphicsBackend + factory create_backend (choix du backend)
    └── wgpu/        module PRIVÉ — invisible hors de backend/
        ├── mod.rs       WgpuBackend : état GPU, resize, orchestration du rendu
        ├── setup.rs     chaîne d'initialisation (instance → surface → adapter → device)
        ├── frame.rs     acquisition de surface, encodage de la passe (couleur + profondeur), présentation
        ├── pipeline.rs  création des pipelines sous error scope (depth + culling)
        ├── buffer.rs    création/destruction des buffers GPU (pool générationnel)
        ├── uniforms.rs  layouts group(0) frame / group(1) objet, slots par draw
        ├── depth.rs     texture/vue de profondeur (Depth32Float)
        └── convert.rs   frontière de traduction (couleurs, formats, matrices, erreurs)
chaos_renderer/shaders/
└── vertex_color.wgsl    shader builtin chaos.vertex_color (groups 0/1, P·V·M)
chaos_renderer/tests/
├── isolation.rs             le verrou : échoue si wgpu apparaît hors de backend/
└── shader_validation.rs     le verrou naga : chaque WGSL embarqué doit compiler (nom + position sinon)
```

## Garanties d'isolation

```
Chaos Engine → Renderer API → Graphics Abstraction → Wgpu Backend → wgpu → Metal / DX12 / Vulkan
```

Quatre verrous rendent l'isolation mécanique, pas disciplinaire :

1. **Module privé** : `backend/wgpu` n'est nommable que depuis `backend/` — le compilateur interdit à quiconque d'importer `WgpuBackend`.
2. **Factory unique** : `create_backend` (dans `backend/mod.rs`) est le seul endroit du moteur qui connaît les backends concrets. Un backend maison = une branche de plus dans cette fonction.
3. **Test d'isolation** (`tests/isolation.rs`) : la CI échoue si `wgpu::`/`use wgpu` apparaît hors de `src/backend/`, ou si un autre manifeste que celui de `chaos_renderer` déclare la dépendance. Vérifié par contre-épreuve (fuite témoin détectée et nommée).
4. **Manifeste unique** : la dépendance wgpu n'existe que dans `chaos_renderer/Cargo.toml`.
5. Les erreurs backend sont traduites en `ChaosError::Graphics` à la frontière (`convert::graphics_error`) — aucun type d'erreur wgpu ne remonte.

## Feuille de route Rendering Core V1 — où atterrit chaque sous-étape

| Sous-étape | Destination | Statut |
|---|---|---|
| 1. Architecture | carte des modules, testabilité au mock | ✅ |
| 2. Backend isolé | factory + module privé + test d'isolation | ✅ |
| 3. Frame lifecycle | `frame.rs` + cycle explicite `backend/wgpu/frame.rs` | ✅ |
| 4. Pipeline minimal | `resources/pipeline.rs` + `backend/wgpu/pipeline.rs` + draws dans le plan | ✅ |
| 5. Shader system V1 | `shaders/*.wgsl` + `src/shaders.rs` (bibliothèque) + validation naga | ✅ |
| 6. Buffers GPU | `resources/buffer.rs` + pool générationnel `backend/wgpu/{pool,buffer}.rs` | ✅ |
| 7. Premier triangle | Renderer-service dans EngineContext, `ColorVertex`, buffer lié au draw, démo côté sandbox | ✅ |
| 8. Géométrie basique | `geometry.rs` (données CPU + primitives) + chemin indexé | ✅ |
| 9. Mesh abstraction | `mesh.rs` + registre dans le Renderer, DrawCommand = pipeline + mesh | ✅ |
| 10. Vertex format | layouts déclaratifs (`VertexLayout::packed`), conversion dynamique backend | ✅ |

**Rendering Core V1 : 10/10 — complet.**

## Feuille de route Rendering Core V2

| Sous-étape | Destination | Statut |
|---|---|---|
| 1. Transform System | `chaos_core::{math, transform}` — glam devient la fondation mathématique | ✅ |
| 2. Math Conventions | `docs/architecture/math-conventions.md` + constantes `math::world` + tests de verrouillage | ✅ |
| 3. Uniform Management | convention group(0)=frame / group(1)=objet, slots par draw, `backend/wgpu/uniforms.rs` | ✅ |
| 4. Camera | `chaos_core::Camera` (view = inverse du transform, projection bénie, `set_viewport`) + `Renderer::surface_size` | ✅ |
| 5. Debug Camera Controller | `chaos_engine::debug::DebugCameraController` (drag droit + WASD physiques, molette vitesse) | ✅ |
| 6. Depth Buffer | `backend/wgpu/depth.rs` — attachement de profondeur dans la passe, test Less sur tous les pipelines | ✅ |
| 7. Cube 3D | `Geometry::cube` (24 sommets, couleur par face, CCW extérieur) + premier Transform non-identité + back-face culling | ✅ |
| 8. Multiple Objects | preuve N objets : mesh partagé × N draws, transforms par frame, slots d'uniforms réutilisés — zéro code moteur nouveau | ✅ |
| 9. Render Queue V1 | `queue.rs` — `RenderQueue`, tri stable par pipeline, contrat « plan déjà en ordre de rendu » | ✅ |
| 10. Validation V2 | audit complet (code sain, isolation, docs), matrice portes + runs réels | ✅ |

**Rendering Core V2 : 10/10 — complet.** Chaos Engine rend une vraie scène 3D : caméra perspective pilotable, transforms par objet, profondeur, N objets organisés par une RenderQueue.

Note : les maths du moteur passent par le point unique `chaos_core::math` (re-export glam). Conventions implémentées dès l'étape 1 (main droite, +Y haut, -Z avant), verrouillées à l'étape 2.

## La couture avec la fenêtre : raw-window-handle

`chaos_renderer` ne dépend pas de `chaos_window` (règle d'architecture : sous-systèmes → core uniquement). Le pont est le standard d'interop `raw-window-handle` :

- `chaos_window::WindowHandle` implémente `HasWindowHandle`/`HasDisplayHandle` (délégation à winit) ;
- `chaos_renderer::SurfaceTarget` accepte toute cible exposant ces handles (impl blanket) ;
- seul `chaos_engine`, qui voit les deux crates, passe la fenêtre au renderer (`RenderSubsystem`).

## Intégration au cycle de vie du moteur

Le **Renderer est un service de l'`EngineContext`** : l'Engine le crée à l'ouverture de la fenêtre (un échec GPU emprunte le chemin d'erreur d'init → arrêt propre) et tout subsystem y accède via `context.renderer_mut()` — c'est ainsi que le contenu (démos, futurs gamemodes, systèmes ECS) crée ses pipelines/buffers et soumet ses draws, sans jamais voir le bas niveau. `renderer()` vaut `None` hors fenêtre : l'API des subsystems reste testable sans GPU.

Le `RenderSubsystem` est un **pilote sans état**, enregistré automatiquement en dernier :

| Hook | Action |
|---|---|
| `on_event` `Resized` | `renderer.resize(w, h)` via le contexte |
| `render` | `renderer.render_frame()` — draine les draws accumulés pendant la phase update (erreur fatale → `request_exit`) |
| `shutdown` | retire le Renderer du contexte (le GPU meurt en premier, ordre inverse) |

Le rendu est piloté par `RedrawRequested` (hook `on_redraw` → phase render), pas par la boucle d'update — indispensable pour rester fluide pendant le resize interactif macOS. `on_update` se termine par `request_redraw()`.

## Cycle d'une frame

L'abstraction décrit le **quoi** (`FramePlan` : clear color aujourd'hui, draw calls demain) ; le backend exécute le **comment** à travers un cycle explicite, et rend compte via `FrameOutcome` :

```
Renderer::render_frame()
  ├─ construction du FramePlan (ordre de la RenderQueue + état du renderer)
  └─ GraphicsBackend::render(plan)
       ├─ garde zéro-aire            fenêtre 0×0 → Skipped(ZeroArea)
       ├─ acquire_frame()            acquisition de la texture de surface
       ├─ encode_frame(view, plan)   encoder → begin pass (clear) → [futurs draw calls] → end pass
       ├─ submit_and_present()       queue.submit puis queue.present
       └─ FrameOutcome::Rendered
```

### Cas dégradés, tous gérés et observables

| Situation | Réaction du backend | Outcome remonté |
|---|---|---|
| Frame acquise (`Success`/`Suboptimal`) | encode + present | `Rendered` |
| Surface perdue/obsolète (`Lost`/`Outdated`) | reconfiguration immédiate, frame suivante saine | `Skipped(SurfaceReconfigured)` |
| Fenêtre occluse ou timeout (`Timeout`/`Occluded`) | frame sautée, aucun travail GPU | `Skipped(SurfaceUnavailable)` |
| Fenêtre réduite à 0×0 (`resize(0,0)`, minimisation) | rendu suspendu sans toucher la surface, réveillé au resize valide | `Skipped(ZeroArea)` |
| Erreur de validation à l'acquisition | erreur fatale traduite (`ChaosError::Graphics`) → arrêt propre du moteur | `Err(...)` |

Le shutdown est la phase terminale du cycle : le `RenderSubsystem` (premier détruit à l'arrêt, ordre inverse) droppe le `Renderer`, libérant surface, device et queue.

## Pipelines

Le pipeline est un concept du moteur, jamais un type wgpu :

```
PipelineDescriptor ──► Renderer::create_pipeline ──► PipelineHandle (opaque, Copy)
   label, ShaderSource (WGSL),                            │
   entrées vs/fs, topology,                               ▼
   cull, front face                    DrawCommand { pipeline, vertex_count }
                                             │  Renderer::queue_draw
                                             ▼
                                   FramePlan.draws → exécutés dans la passe
```

- **WGSL est le langage shader officiel du moteur** (`ShaderSource::Wgsl`) — compilable vers SPIR-V via naga pour un futur backend maison ; l'enum accueillera d'autres formats.
- **Vertex layouts déclaratifs** : `PipelineDescriptor.vertex_layout: Option<VertexLayout>` (`None` = bufferless). Le layout est défini côté Chaos (`VertexAttributeFormat`, `VertexAttribute { location, format, offset }`, `step_mode Vertex|Instance` — l'instancing est préparé) et converti vers wgpu uniquement dans le backend. `VertexLayout::packed(&[formats])` calcule locations/offsets/stride ; `ColorVertex::layout()` décrit le vertex standard via ce système. UV/normales/tangentes/skinning = des attributs de plus ; un seul slot de layout pour l'instant (multi-slots avec l'instancing).
- La cible couleur est implicitement le **format de la surface** (résolu par le backend) ; blend REPLACE — les cibles offscreen et le blending configurable viendront avec leurs phases. Tous les pipelines écrivent et testent la **profondeur** (voir ci-dessous).
- **Culling** : `CullMode::None` par défaut ; `.with_cull_mode(CullMode::Back)` est le réglage standard des pipelines 3D opaques. Il repose sur la convention d'enroulement **CCW vu de l'extérieur** (`docs/architecture/math-conventions.md`) et rend les géométries 2D single-sided. La démo emploie les deux : pipeline cullé pour les cubes fermés, pipeline double-sided (le défaut) pour ses formes plates (sol, triangles).
- Côté backend (`backend/wgpu/pipeline.rs`) : création sous **error scope wgpu** — un WGSL invalide ou un pipeline incohérent devient une `ChaosError::Graphics` propre, jamais un panic. Stockage en `Vec`, handle = index (suppression et générations viendront avec la gestion de ressources).
- Exécution : `encode_frame` rejoue les `FrameDraw` du plan dans la passe ; un handle inconnu est ignoré avec un `warn!`, jamais de panic.
- Les draws soumis vivent dans la **RenderQueue** (section dédiée ci-dessous) avec la **durée de vie d'une frame de simulation** : le moteur la vide au début de chaque update (`clear_draws`), et toutes les présentations intermédiaires (rafales de redraw du resize interactif) re-présentent la même file — jamais de frame vide entre deux updates. Le scene graph alimentera cette file plus tard.

## Render Queue

Les draws soumis via `Renderer::queue_draw` alimentent la **`RenderQueue`** (`queue.rs`) — le concept qui transforme une succession de draw calls improvisés en rendu organisé :

- **Contrat** : la queue reçoit les soumissions en **ordre de scène** et rend l'**ordre de rendu** (`ordered()`) ; le `FramePlan` arrive au backend **déjà trié** et le backend exécute aveuglément. La politique (l'ordre) appartient au moteur, la mécanique (l'exécution) au backend.
- **Clé V1 : le pipeline** — tri **stable** (`sort_by_key`) : le regroupement par pipeline minimise les changements d'état GPU, et l'ordre de soumission est préservé à clé égale (déterminisme). Le tri est légal car la géométrie est opaque : le depth buffer rend l'ordre d'exécution invisible à l'écran.
- Premier bénéfice concret : le backend saute les `set_pipeline` redondants — la scène démo (13 draws, 2 pipelines) fait 2 binds au lieu de 13.
- **Extensions prévues** — la clé grandit, le contrat ne change pas : passes de rendu, opaque/transparent (tri par profondeur), matériaux, ombres, debug rendering ; optimisations notées : buckets/dirty-flag, skip des binds de buffers (mesh partagé), instancing, dynamic offsets.
- Pure structure CPU, testée sans GPU (stabilité, regroupement, cycle de vie).

## Profondeur

Le depth buffer est de la **pure mécanique de backend** : ni `FramePlan`, ni `Renderer`, ni le trait `GraphicsBackend`, ni les shaders n'en savent rien — l'occlusion 3D est devenue correcte sans toucher un seul type public.

- **Format** : `Depth32Float` (`backend/wgpu/depth.rs`), le format profondeur portable de référence.
- **Cycle de vie** : la texture suit la surface — créée à l'init, recréée au resize (la garde 0×0 suspend le rendu avant d'y arriver), droppée avec le backend. `Lost`/`Outdated` reconfigurent la surface sans changer les dimensions : la vue reste valide.
- **La passe** : clear à `1.0` chaque frame (le plus lointain — profondeur wgpu 0..1, nos conventions), `store: Discard` — personne ne relit la profondeur pour l'instant, optimal sur GPU tile-based.
- **Les pipelines** : écriture activée, comparaison `Less` (plus proche = plus petit). Les pipelines sans test de profondeur (UI, post-process) arriveront avec leur premier consommateur ; le **reverse-Z** est noté comme future optimisation de précision.

## Shaders

Cinq réponses, une organisation minimale mais durable :

| Question | Réponse |
|---|---|
| Où ils vivent | `chaos_renderer/shaders/*.wgsl` — de vrais fichiers, embarqués à la compilation (`include_str!`), zéro I/O runtime |
| Comment identifiés | `ShaderLibrary` : noms nommespacés (`chaos.` pour les intégrés), constantes `shaders::builtin::*` — jamais de littéraux éparpillés |
| Comment chargés | `with_builtins()` charge les intégrés ; `register()` pour matériaux/post-process/jeux ; `PipelineDescriptor.shader` est un `ShaderRef` (`Named` résolu via la bibliothèque, `Inline` pour le prototypage) ; le backend reçoit la source déjà résolue |
| Comment les erreurs remontent | nom inconnu → `ChaosError::Graphics` explicite avant tout appel GPU ; WGSL invalide → test de validation **naga** en CI (message avec nom + ligne/colonne) ; création GPU → error scopes avec label |
| Comment le futur s'y branche | le shader compiler/asset pipeline remplacera le *chargement*, pas l'organisation ; hot-reload via la bibliothèque ; naga (dev-dependency) est déjà l'outil du futur compiler WGSL → SPIR-V |

Le langage shader officiel du moteur est **WGSL** (`ShaderSource::Wgsl`). naga n'apparaît qu'en dev-dependency (tests) — jamais dans l'API.

## Buffers

Ressources de données GPU, dans le vocabulaire du moteur :

- **`BufferDescriptor`** (`label`, `kind: Vertex | Index`, `contents: Vec<u8>`) — les données sont uploadées à la création (buffers immutables ; `write_buffer` dynamique viendra avec ses besoins). Helpers `vertex()`/`index()` + `bytes_of_f32` (endianness native, zéro dépendance).
- **`BufferHandle` générationnel** — le cœur de la gestion de durée de vie : les slots du pool backend sont réutilisés mais chaque réutilisation incrémente la génération. Un handle périmé est **détecté** : `destroy_buffer` → erreur explicite (« stale or already destroyed »), un accès en rendu → ignoré avec warn. Jamais de résolution silencieuse vers une autre ressource.
- **Destruction propre, deux chemins** : `destroy_buffer` explicite (retrait du pool + drop), ou drop du backend au shutdown (tout ce qui reste est libéré — wgpu gère la libération différée côté GPU).
- Le pool (`backend/wgpu/pool.rs`) est **générique et testé sans GPU** ; il servira aux pipelines quand ils deviendront destructibles (pour l'instant `PipelineHandle` reste un index simple — rien ne se détruit).
- À venir : vertex layouts déclaratifs (étape 10), uniform buffers avec les bind groups.

## Uniforms

Le moteur parle en matrices et Transforms — jamais en bind groups. Convention de binding du moteur (généralisable : matériaux → group 2) :

| Groupe WGSL | Contenu | Fréquence | Mécanique backend |
|---|---|---|---|
| `@group(0)` | `FrameUniforms { view_projection }` | 1× par frame | buffer 64 o unique, `queue.write_buffer` |
| `@group(1)` | `ObjectUniforms { model }` | 1× par draw | pool de slots (buffer + bind group), réutilisés par index de draw, agrandi à la demande |

- Côté abstraction : `Renderer::set_view_projection(Mat4)` (la caméra le fournit) et `DrawCommand.transform: Transform` (résolu en matrice modèle au plan). Le trait backend n'a gagné aucune méthode : les uniforms sont de la mécanique interne pilotée par le plan.
- Tous les pipelines reçoivent le layout standard `[frame, objet]` ; `mat4_to_bytes` convertit sans allocation (column-major glam = layout WGSL).
- Optimisation prévue pour le render queue : dynamic offsets sur un buffer unique au lieu d'un slot par draw.

## Géométrie

La géométrie est une **donnée moteur**, distincte de sa représentation GPU et de son usage :

| Couche | Type |
|---|---|
| Données CPU | `Geometry` (`geometry.rs`) : `Vec<ColorVertex>` + indices u16 (vide = non indexé) ; constructeurs `triangle(center, size, colors)` / `quad(center, w, h, color)` / `cube(center, size, face_colors)` — debug lines, sphères, etc. seront des constructeurs de plus |
| Représentation GPU | buffers créés depuis `vertex_bytes()`/`index_bytes()` (étape 6) |
| Usage | `DrawCommand { pipeline, vertex_buffer, index_buffer, element_count }` — `index_buffer` présent → `draw_indexed` (Uint16), sinon `draw` |

Le cube est la première géométrie **fermée** : 24 sommets (4 par face — une couleur par face exige des sommets non partagés, la topologie qu'exigeront normales/UVs), 36 indices, faces ordonnées **+X, -X, +Y, -Y, +Z, -Z**, enroulement **CCW vu de l'extérieur** (convention verrouillée par test — voir `docs/architecture/math-conventions.md`). Depuis l'étape 8, toute géométrie de la démo est construite **à l'origine** et placée exclusivement par le `Transform` de son `DrawCommand` ; le paramètre `center` des constructeurs reste disponible pour cuire un offset local quand c'est pertinent.

## Meshes

Le mesh est la **ressource de rendu de première classe** du moteur — c'est elle que consommeront asset system, scènes, ECS, éditeur et l'API de contenu (primitives aujourd'hui ; glTF, assets importés, outils et contenu utilisateur demain : tous aboutiront à `create_mesh`).

```
Geometry (données CPU) ──► Renderer::create_mesh(label, &geometry) ──► MeshHandle
                                    │ le mesh POSSÈDE ses buffers GPU
DrawCommand { pipeline, mesh } ─────┤ Renderer::queue_draw
                                    ▼ résolution au render_frame (registre générationnel)
                       FrameDraw { buffers, element_count } ──► backend (inchangé)
```

- **Le mesh vit dans l'abstraction** : le backend ne connaît toujours que buffers et pipelines. Le `Renderer` tient le registre (pool générationnel partagé, `src/pool.rs`) et résout mesh → buffers en construisant le plan.
- **Un mesh = une ressource, un draw = un usage** : le même `MeshHandle` peut être soumis N fois par frame avec des transforms différents — mêmes buffers GPU, une matrice modèle par draw (slot d'uniform par index, réutilisé chaque frame). Verrouillé par test mock ; la ronde de la démo dessine 8 cubes d'un seul mesh. L'instancing GPU sera l'optimisation de ce motif.
- **Durée de vie** : `destroy_mesh` détruit le record ET ses buffers ; handle périmé → erreur explicite ; un draw sur mesh détruit est écarté du plan avec un `warn!`, jamais de panic.
- Le record porte déjà le **vertex format** (`VertexLayout`) ; les **bounds** (AABB) s'y logeront quand le culling en aura besoin. La validation croisée pipeline↔mesh est notée pour plus tard.
- `create_buffer`/`destroy_buffer` restent publics pour les usages avancés, mais les apps parlent mesh.

Présentation : `AutoVsync` si `RendererConfig::vsync` est actif, `AutoNoVsync` sinon — le défaut du moteur est vsync **off**, car un present bloquant sur le main thread rend les interactions fenêtre laggy sur macOS (winit #1737) ; la cadence est régulée par `target_fps` côté moteur. Delta d'horloge borné côté moteur.

## Couleur

`chaos_core::Color` (RGBA f32 linéaire) est le type du vocabulaire moteur ; la conversion vers le type du backend se fait à la frontière (`to_wgpu_color`). `EngineConfig::clear_color` contrôle la couleur de fond.

## Ce que les phases futures brancheront ici

- **Triangle / pipelines** : pipeline de rendu, shaders (WGSL), vertex buffers — extension du contrat `GraphicsBackend`.
- **Meshes, textures, matériaux, caméra, lumières** : nouvelles ressources exposées par `Renderer`, implémentées derrière le trait.
- **Post-process, render graph** : orchestration au niveau `Renderer`, opaque pour le moteur.
- **Mode headless** (serveur dédié) : un backend nul derrière le même trait.
