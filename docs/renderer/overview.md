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
├── lib.rs           façade publique : Renderer, RendererConfig, GraphicsBackend, SurfaceTarget
├── config.rs        RendererConfig — paramètres d'attachement (dimensions, vsync)
├── renderer.rs      Renderer — orchestrateur haut niveau (+ tests avec backend factice)
├── target.rs        SurfaceTarget — couture raw-window-handle avec la fenêtre
└── backend/
    ├── mod.rs       trait GraphicsBackend + factory create_backend (choix du backend)
    └── wgpu/        module PRIVÉ — invisible hors de backend/
        ├── mod.rs       WgpuBackend : état GPU + rendu de frame
        ├── setup.rs     chaîne d'initialisation (instance → surface → adapter → device)
        └── convert.rs   frontière de traduction (couleurs, handles, erreurs)
chaos_renderer/tests/
└── isolation.rs     le verrou : échoue si wgpu apparaît hors de backend/
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

Ces fichiers n'existent pas encore : ils naissent avec leur sous-étape, jamais en stub vide.

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
  ├─ construction du FramePlan (état du renderer)
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
- La cible couleur est implicitement le **format de la surface** (résolu par le backend) ; blend REPLACE, pas de depth — les cibles offscreen, le blending configurable et le depth viendront avec leurs phases.
- Côté backend (`backend/wgpu/pipeline.rs`) : création sous **error scope wgpu** — un WGSL invalide ou un pipeline incohérent devient une `ChaosError::Graphics` propre, jamais un panic. Stockage en `Vec`, handle = index (suppression et générations viendront avec la gestion de ressources).
- Exécution : `encode_frame` rejoue les `DrawCommand` du plan dans la passe (`set_pipeline` + `draw`) ; un handle inconnu est ignoré avec un `warn!`, jamais de panic.
- La file de draws du `Renderer` est vidée à chaque frame (immediate mode) — le scene graph la remplira plus tard.

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

## Géométrie

La géométrie est une **donnée moteur**, distincte de sa représentation GPU et de son usage :

| Couche | Type |
|---|---|
| Données CPU | `Geometry` (`geometry.rs`) : `Vec<ColorVertex>` + indices u16 (vide = non indexé) ; constructeurs `triangle(center, size, colors)` / `quad(center, w, h, color)` — cube et debug lines seront des constructeurs de plus |
| Représentation GPU | buffers créés depuis `vertex_bytes()`/`index_bytes()` (étape 6) |
| Usage | `DrawCommand { pipeline, vertex_buffer, index_buffer, element_count }` — `index_buffer` présent → `draw_indexed` (Uint16), sinon `draw` |

Le « center » est cuit dans les sommets en attendant les transformations/caméra (uniforms).

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
- **Durée de vie** : `destroy_mesh` détruit le record ET ses buffers ; handle périmé → erreur explicite ; un draw sur mesh détruit est écarté du plan avec un `warn!`, jamais de panic.
- Le record porte déjà le **vertex format** (`VertexInput`) ; les **bounds** (AABB) s'y logeront quand le culling en aura besoin. La validation croisée pipeline↔mesh est notée pour plus tard.
- `create_buffer`/`destroy_buffer` restent publics pour les usages avancés, mais les apps parlent mesh.

Présentation : `AutoVsync` si `RendererConfig::vsync` est actif, `AutoNoVsync` sinon — le défaut du moteur est vsync **off**, car un present bloquant sur le main thread rend les interactions fenêtre laggy sur macOS (winit #1737) ; la cadence est régulée par `target_fps` côté moteur. Delta d'horloge borné côté moteur.

## Couleur

`chaos_core::Color` (RGBA f32 linéaire) est le type du vocabulaire moteur ; la conversion vers le type du backend se fait à la frontière (`to_wgpu_color`). `EngineConfig::clear_color` contrôle la couleur de fond.

## Ce que les phases futures brancheront ici

- **Triangle / pipelines** : pipeline de rendu, shaders (WGSL), vertex buffers — extension du contrat `GraphicsBackend`.
- **Meshes, textures, matériaux, caméra, lumières** : nouvelles ressources exposées par `Renderer`, implémentées derrière le trait.
- **Post-process, render graph** : orchestration au niveau `Renderer`, opaque pour le moteur.
- **Mode headless** (serveur dédié) : un backend nul derrière le même trait.
