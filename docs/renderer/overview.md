# Renderer — architecture

Référence des choix de la phase 2 (renderer minimal). Le principe directeur : **wgpu est un détail d'implémentation, jamais une dépendance du moteur.**

## Les deux étages

```
chaos_engine ──► Renderer (API orientée moteur, vocabulaire chaos_core)
                   └─► trait GraphicsBackend (le point de remplacement)
                         └─► WgpuBackend (seule zone du moteur qui importe wgpu)
```

- **`Renderer`** (`renderer/`) — ce que voit le moteur : `attach(target, RendererConfig)`, `resize`, `set_clear_color`, `render_frame`, `description`. Ne parle que le vocabulaire de `chaos_core` (`Color`, `ChaosResult`). Testable sans GPU par injection de backend (`with_backend`, interne) — les tests unitaires de la crate vérifient l'orchestration avec un backend factice.
- **`GraphicsBackend`** (`backend/mod.rs`) — le contrat qu'un backend doit honorer. Remplacer wgpu par un backend maison (Vulkan, DirectX 12, Metal) = implémenter ce trait, rien d'autre à toucher dans le moteur.
- **`WgpuBackend`** (`backend/wgpu/`) — l'unique zone du workspace qui importe wgpu. Détient surface, device, queue et configuration.

## Carte des modules

```
chaos_renderer/src/
├── lib.rs           façade publique : Renderer, RenderQueue, Geometry, MeshHandle, descripteurs…
├── capabilities.rs  RendererCapabilities / DeviceLimits — ce que le GPU offre, ce qui en est décidé
├── config.rs        RendererConfig — paramètres d'attachement (dimensions, vsync)
├── debug.rs         DebugDraw / DebugShape / DebugDepth — le debug rendering (lignes monde)
├── diagnostics.rs   RendererDiagnostics / GpuTiming — le snapshot de ce que la frame coûte
├── environment.rs   EnvironmentDescriptor / EnvironmentInfo — cubemap, intensité, ciel
├── renderer/        Renderer — le struct + les types transverses + render_frame/render_to_target
│   ├── mod.rs           l'orchestrateur : définition, construction, coordination de frame
│   ├── pipelines.rs     la fabrique de pipelines — les cinq caches de permutations
│   ├── resolve.rs       le cœur chaud : file → plan (opacité, culling, batching, moisson d'ombre)
│   ├── resources.rs     buffers, textures, samplers, cibles — limites, retraite, stats
│   ├── materials.rs     création validée, mises à jour in-place, destruction
│   ├── meshes.rs        la géométrie matérialisée en buffers + ses bounds
│   ├── lighting.rs      lumières, ambiante, environnement, exposition, réglages d'ombre
│   ├── passes.rs        le registre des passes déclarées et leurs files
│   ├── debug_draws.rs   le store du debug rendering et sa résolution par passe
│   ├── instrumentation.rs  l'analyse des draws, la clôture des diagnostics, le budget CPU
│   └── tests/           les tests white-box par domaine (support.rs + 13 fichiers, 201 tests)
├── frame.rs         DrawCommand / FrameDraw / FramePlan / FrameOutcome / FrameSkipReason
├── queue.rs         RenderQueue — ordre de rendu (tri stable par pipeline)
├── geometry.rs      Geometry + TexturedGeometry — données CPU, constructeurs triangle / quad / cube
├── lifetime.rs      LifetimeTracker — registre de durée de vie (états, dépendances, retraite, stats)
├── light.rs         Light (Directional/Point/Spot) / MAX_LIGHTS / FrameLights — l'éclairage par frame
├── material.rs      MaterialModel / MaterialOpacity / MaterialDescriptor / MaterialInfo — la couche visuelle
├── mesh.rs          MeshHandle + MeshRecord (le mesh possède ses buffers + ses bounds locaux)
├── pass.rs          RenderPassDescriptor / PassHandle / PassLoad + FrameReport — les passes déclarées
├── pool.rs          ResourcePool générationnel (privé) — détection des handles périmés
├── shaders.rs       ShaderLibrary + noms builtin (chaos.*) + convention inputs (groupes/slots)
├── shadow.rs        DirectionalShadowDescriptor / ShadowVolume / light_view_projection — les ombres
├── suite.rs         (test) la SUITE stress & régression — scène canonique, quatre familles
├── target.rs        SurfaceTarget — couture raw-window-handle avec la fenêtre
├── testing.rs       (test) le banc d'essai partagé — MockBackend à journal, issue commutable
├── visibility.rs    Frustum — le frustum par vue et son test de bounds (le culling)
├── resources/       vocabulaire des ressources, indépendant du backend
│   ├── binding.rs       MaterialBindingDescriptor / MaterialBindingHandle (le groupe 2 vu du backend)
│   ├── buffer.rs        BufferDescriptor / BufferHandle / BufferKind + bytes_of_*
│   ├── pipeline.rs      PipelineDescriptor / PipelineHandle / topology / cull / color_target / transparent
│   ├── render_target.rs RenderTargetDescriptor / RenderTargetHandle (cibles hors écran)
│   ├── sampler.rs       SamplerDescriptor / SamplerHandle / filtre / adressage
│   ├── shader.rs        ShaderRef (Named | Inline) / ShaderSource (Wgsl)
│   ├── texture.rs       TextureDescriptor / TextureHandle / TextureFormat / TextureUsage
│   └── vertex.rs        VertexLayout / VertexAttribute / ColorVertex / TexturedVertex
└── backend/
    ├── mod.rs       trait GraphicsBackend + factory create_backend (choix du backend)
    └── wgpu/        module PRIVÉ — invisible hors de backend/
        ├── mod.rs       WgpuBackend : état GPU, resize, orchestration du rendu
        ├── setup.rs     chaîne d'initialisation (instance → surface → adapter → device)
        ├── frame.rs     acquisition de surface, encodage d'UNE passe, règles load/store de profondeur
        ├── pipeline.rs  création des pipelines sous error scope (depth + culling + cible + blend)
        ├── render_target.rs  cibles hors écran : couleur DANS le pool textures, vues + profondeur propres
        ├── binding.rs   layout + bind groups material (texture/sampler/uniforms, pool générationnel)
        ├── buffer.rs    création/destruction des buffers GPU (pool générationnel)
        ├── sampler.rs   création/destruction des samplers GPU (pool générationnel)
        ├── texture.rs   création/upload/destruction des textures GPU (pool générationnel)
        ├── uniforms.rs  layouts group(0) frame / group(1) objet, slots par draw
        ├── instances.rs buffer d'instances croissant (128 o/instance, partagé entre passes)
        ├── debug.rs     buffer des sommets de debug croissant (28 o/sommet, partagé)
        ├── timing.rs    GpuTimer — timestamp queries, ring de readbacks, jamais bloquant
        ├── depth.rs     texture/vue de profondeur (Depth32Float)
        └── convert.rs   frontière de traduction (couleurs, formats, matrices, erreurs)
chaos_renderer/shaders/
├── vertex_color.wgsl    shader builtin chaos.vertex_color (groups 0/1, P·V·M)
├── textured.wgsl        shader builtin chaos.textured (groups 0/1/2, sampling material)
├── lit.wgsl             shader builtin chaos.lit (éclairage Lambert + ombres)
├── pbr.wgsl             shader builtin chaos.pbr (metallic-roughness + IBL + ombres)
├── sky.wgsl             shader builtin chaos.sky (fond cubemap, profondeur max)
├── debug.wgsl           shader builtin chaos.debug (lignes monde, couleur par sommet)
└── shadow.wgsl          shader builtin chaos.shadow (profondeur seule, vertex uniquement)
chaos_renderer/tests/
├── isolation.rs             le verrou : échoue si wgpu apparaît hors de backend/
└── shader_validation.rs     le verrou naga : chaque WGSL embarqué doit compiler (nom + position sinon)
```

## La frontière publique — stabilisée (consolidation, sous-phase 1)

L'API publique est le CONTRAT que l'éditeur et les futurs systèmes consommeront. Elle est auditée, documentée à 100 % et verrouillée : `#![deny(missing_docs)]` — tout item public non documenté casse le build, pour toujours.

**Deux audiences, énoncées dans la doc de crate (`lib.rs`)** :

| Audience | Types | Rôle |
|---|---|---|
| le CONSOMMATEUR (moteur, futur éditeur) | `Renderer`, descripteurs, handles, `DrawCommand`, géométries, `ShaderLibrary` | décrire des ressources, soumettre des draws — jamais un détail d'implémentation |
| l'IMPLÉMENTEUR DE BACKEND (futur Vulkan/DX12/Metal natif) | `GraphicsBackend`, `FramePlan`/`FrameDraw`, `SurfaceTarget` | exécuter le plan de frame, en vocabulaire Chaos exclusivement |

**Les garanties de la frontière** :
- handles OPAQUES (champs `pub(crate)`) et GÉNÉRATIONNELS (`ResourcePool`) — un handle périmé est détecté, jamais résolu vers une autre ressource ;
- descripteurs 100 % backend-agnostic (enums maison : `TextureFormat`, `CullMode`, …) ;
- validation sémantique AVANT le backend ; erreurs `ChaosError::Graphics`, zéro `unwrap`/`panic` hors tests ;
- limites V1 ASSUMÉES et écrites : pipelines permanents (pas de destruction, handle non générationnel — la gestion mémoire viendra avec sa sous-phase), mutualisation des textures par nom (`get_or_create_texture`).

## La durée de vie des ressources (consolidation, sous-phase 2)

Le renderer CONNAÎT ses ressources — identité, état, dépendances, coût. Le registre (`lifetime.rs`) vit côté Renderer, backend-agnostic ; il travaille aux créations/destructions, jamais sur le chemin chaud d'un draw.

**Les états** : `Alive` (dans le modèle) → `Retired` (sortie du modèle : handle mort, refus périmés immédiats — mais libération backend EN ATTENTE) → libérée au point sûr : la fin du `render_frame` suivant. wgpu garantit déjà la survie des ressources en vol ; la retraite fixe le CONTRAT du point de libération pour les futurs backends natifs — observable (`stats.retired`), testé par le journal du backend factice.

**Les règles de dépendance (l'ordre incorrect est un REFUS, jamais un effet silencieux)** :

| Destruction demandée | Règle |
|---|---|
| texture partagée par N materials | refusée en nommant le compte (« still used by N material(s) ») |
| sampler partagé | idem |
| buffer possédé par un mesh | refusée en nommant le mesh (« destroy the mesh instead ») |
| fallback builtin (`chaos.white`, `chaos.default_sampler`) | refusée (« builtin fallback ») — les fallbacks sont PROTÉGÉS |
| handle périmé / double destruction | erreur explicite « stale or already destroyed » |
| mesh | emporte SES buffers (le propriétaire emporte ses organes) |
| material | rend ses parts (texture/sampler redeviennent destructibles au dernier partageur), son binding part en retraite |

**Les statistiques** (`Renderer::resource_stats()`) : comptes vivants par famille, octets EXACTS des buffers et textures (les octets uploadés — l'estimation exploitable), retraites en attente, total estimé. La famille `render_targets` compte les cibles hors écran depuis la sous-phase 4 (octets = la profondeur ; la couleur est comptée avec les textures). Extensible aux ressources d'animation (mêmes familles). Le branchement aux metrics moteur viendra avec la sous-phase diagnostics CPU/GPU.

**Nettoyage** : le drop du Renderer (shutdown moteur, `take_renderer`) libère tout — wgpu relâche les objets GPU ; la retraite n'y survit pas. Limite V1 : les pipelines sont permanents (comptés, jamais détruits).

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

## Statut : Rendering Core MATURE V1 — consolidation 18/18 CLOSE

**La Rendering Core Consolidation (sous-phases 1–18) est validée : le
renderer est une fondation graphique mature V1 et une dépendance STABLE
pour Chaos Editor et les systèmes de Chaos Engine 1.0.** L'audit final —
la matrice des preuves par domaine, les 13 attestations d'architecture
vérifiées, le REGISTRE DE LA DETTE V1 (chaque approximation assumée et
son chemin de remboursement), les runs consignés (zéro dérive mémoire
sur 1 800 frames réelles, GPU mesuré) et la déclaration de maturité —
vit dans **`docs/renderer/consolidation-validation.md`**.

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

## Feuille de route Rendering Core V3

| Sous-étape | Destination | Statut |
|---|---|---|
| 1. Texture Concept | `resources/texture.rs` (descripteur, formats, usages, handle générationnel) + `backend/wgpu/texture.rs` (création/upload, pool) | ✅ |
| 2. Texture Descriptors | `TextureDescriptor::validate()` — le descripteur est l'autorité de sa cohérence, point d'ancrage des règles mips/cubemaps | ✅ |
| 3. GPU Texture Backend | `backend/wgpu/texture.rs` durci (arithmétique saturée, zéro panic) — preuve visuelle sur GPU réel au premier consommateur (samplers/bindings) | ✅ |
| 4. Texture Upload | versant CPU : `rgba8_bytes_of` / `srgb8_bytes_of` (règle sRGB de référence, alpha linéaire) — l'upload GPU existait depuis l'étape 1 | ✅ |
| 5. Sampler Concept | `resources/sampler.rs` (filtre Nearest/Linear, adressage Repeat/Clamp) + `backend/wgpu/sampler.rs` | ✅ |
| 6. Resource Binding V1 | `resources/binding.rs` — TextureBinding au groupe(2), pipelines `with_texture_binding`, DrawCommand.binding ; premier sol texturé dans la démo | ✅ |
| 7. Shader Inputs | `shaders::inputs` — l'autorité exécutable des groupes/slots, consommée par le backend, verrouillée par test naga | ✅ |
| 8. Textured Shader | builtin `chaos.textured` (validé naga) + `TexturedVertex` (position + UV) + `TexturedGeometry::quad` + `create_textured_mesh` — le shader d'app provisoire disparaît | ✅ |
| 9. Textured Pipeline | réalisé à l'étape 8 (pipeline `chaos.textured` + UV + uniforms + depth sur Metal) ; re-validé ici avec un second consommateur | ✅ |
| 10. UV Support | `TexturedGeometry::cube` (24 sommets, UV 0..1 par face, CCW) — le cube central de la démo est texturé | ✅ |
| 11. Material Concept V1 | `material.rs` — le concept de surface : pipeline + base_color + texture/sampler optionnels (fallbacks builtin) ; `DrawCommand` devient mesh + material + transform | ✅ |
| 12. Material Binding | contrôle propre livré à l'étape 11 (résolution, possession, error scopes) + bind material unique par run de material dans la passe | ✅ |
| 13. Texture Cache | `get_or_create_texture` — déduplication par clé logique (label), éviction au destroy, fallback auto-réparant ; la clé accueillera le chemin d'asset | ✅ |
| 14. Lighting Preparation | `docs/renderer/lighting-preparation.md` — la carte d'atterrissage vérifiée de Lighting V1 (zéro code : la préparation est la forme de l'architecture) | ✅ |
| 15. PBR Preparation | section « Material PBR — le plan d'évolution » du même document — slots fixes + fallbacks neutres, shaders valides sous layout élargi, descripteur additif | ✅ |
| 16. Validation V3 | audit complet (code sain, isolation, docs), matrice portes + runs réels | ✅ |

**Rendering Core V3 : 16/16 — complet.** Le renderer gère de vraies ressources graphiques : textures (upload sRGB correct, cache par clé logique), samplers, bindings conventionnés et verrouillés, materials haut niveau avec fallbacks — prêt à être nourri par l'Asset Pipeline.

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

L'abstraction décrit le **quoi** (`FramePlan` : la suite ORDONNÉE des passes de la frame — section « L'orchestration des passes ») ; le backend exécute le **comment** à travers un cycle explicite, et rend compte via `FrameOutcome` :

```
Renderer::render_frame()
  ├─ tri déterministe des passes déclarées (order, puis enregistrement)
  ├─ par passe : les draws de SA file, résolus (périmés écartés, feedback écarté)
  │    désactivée → sautée (rapport) ; cible périmée → auto-désactivée (warn unique)
  └─ GraphicsBackend::render(plan)
       ├─ acquisition UNE fois, si le plan contient des passes surface
       │    (garde zéro-aire / indisponible → les passes surface sautées,
       │     les passes cible s'exécutent quand même)
       ├─ PAR PASSE : uniforms écrits → encode_pass (load/store dérivés) → submit
       └─ present UNE fois, après la dernière passe → FrameOutcome
```

`FrameOutcome` est l'issue de la PRÉSENTATION : `Rendered` = le travail est soumis (et présenté si le plan portait une passe surface) ; `Skipped(raison)` = la présentation a été sautée — les passes cible ont pu s'exécuter quand même. Le détail passe par passe vit dans `Renderer::frame_report()`.

### Cas dégradés, tous gérés et observables

| Situation | Réaction du backend | Outcome remonté |
|---|---|---|
| Frame acquise (`Success`/`Suboptimal`) | encode + present | `Rendered` |
| Surface perdue/obsolète (`Lost`/`Outdated`) | reconfiguration immédiate, frame suivante saine | `Skipped(SurfaceReconfigured)` |
| Fenêtre occluse ou timeout (`Timeout`/`Occluded`) | frame sautée, aucun travail GPU | `Skipped(SurfaceUnavailable)` |
| Fenêtre réduite à 0×0 (`resize(0,0)`, minimisation) | rendu suspendu sans toucher la surface, réveillé au resize valide | `Skipped(ZeroArea)` |
| Erreur de validation à l'acquisition | erreur fatale traduite (`ChaosError::Graphics`) → arrêt propre du moteur | `Err(...)` |

Le shutdown est la phase terminale du cycle : le `RenderSubsystem` (premier détruit à l'arrêt, ordre inverse) droppe le `Renderer`, libérant surface, device et queue.

## Render targets & rendu hors écran (consolidation, sous-phase 4)

Une scène peut être rendue dans une texture de taille configurable, puis cette texture consommée comme n'importe quelle autre — miroirs, écrans de surveillance, minimaps, previews d'éditeur. Le rendu fenêtre est devenu UN cas de destination : le cycle de frame n'a pas été réécrit (l'encodage était déjà agnostique de la vue), la destination est entrée dans le plan.

**La ressource** : `RenderTargetDescriptor { label, width, height, format }` → `create_render_target` → `RenderTargetHandle` (opaque, générationnel). La cible possède SA profondeur (`Depth32Float`, toujours incluse en V1 — les pipelines du moteur attendent un depth-stencil) ; sa COULEUR est une texture du pool, exposée par `render_target_color(handle)` — elle se branche dans un material comme n'importe quelle texture, refcount compris : « l'entrée d'une passe ultérieure » par le mécanisme existant. Tout format échantillonnable est une couleur valide (`Rgba16Float` = offscreen HDR).

**La destination dans le plan** : `FramePlan.destination: RenderDestination { Surface, Target(handle) }`. `Surface` → chemin actuel intact (acquisition, présentation, skips) ; `Target` → vues de la cible, MÊME encodage, submit sans présentation — le rendu hors écran fonctionne même fenêtre minimisée (une cible est toujours disponible ; seul un handle périmé est une erreur).

**L'API** :

```rust
let target = renderer.create_render_target(
    &RenderTargetDescriptor::new("app.mirror", 256, 256, TextureFormat::Rgba8UnormSrgb))?;
renderer.render_to_target(target, clear_color, view_projection, &draws)?;  // rend IMMÉDIATEMENT
let color = renderer.render_target_color(target)?;                         // l'entrée de passe
let target = renderer.resize_render_target(target, 512, 512)?;             // NOUVEAU handle
renderer.destroy_render_target(target)?;
```

- **`render_to_target`** trie ses draws par material (copie locale) et les résout par le MÊME chemin que `render_frame` (draws périmés écartés pareil) — la RenderQueue de la frame principale n'est pas touchée, et la retraite non plus (le point sûr reste la fin du `render_frame`).
- **La règle du format** : un pipeline utilisé vers une cible doit viser le format de la cible — `PipelineDescriptor::with_color_target(format)` (défaut `None` = le format de la surface). Les API graphiques l'exigent ; l'écart est une erreur wgpu claire sous error scope.
- **Resize = rotation générationnelle ASSUMÉE** : `resize_render_target` rend un NOUVEAU handle ; l'ancien handle ET son ancienne couleur deviennent périmés — le consommateur re-résout la couleur et recrée son material (un bind group, ~µs). Détruire d'abord les materials qui échantillonnent la couleur : `destroy_render_target` (et donc le resize) sont REFUSÉS tant qu'elle est partagée (« its color texture is still used by N material(s) »).
- **Durée de vie** : famille `render_targets` des stats (octets = la profondeur ; la couleur est comptée avec les textures) ; à la destruction, la cible et sa couleur partent en retraite différée ensemble.

**Limites V1 assumées** : profondeur obligatoire (pas de cible couleur seule), pas de MRT (le deferred est interdit prématurément), pas de cibles cube (réflexions — plus tard), pas de readback CPU (captures — avec son besoin), pas de MSAA. Le système de passes (section « L'orchestration des passes ») orchestre AU-DESSUS de cette destination : déclarer une passe visant une cible est la voie de la frame orchestrée ; `render_to_target` reste le rendu IMMÉDIAT hors frame (vignettes d'éditeur, bakes).

**La preuve vivante (démo)** : « l'écran de surveillance » — une cible 256×256 rendue chaque frame avec la ronde de cubes vue du dessus (caméra fixe, pipeline offscreen `with_color_target(Rgba8UnormSrgb)`), affichée sur un quad flottant de la scène principale (material = couleur de la cible + sampler ClampToEdge). Depuis la sous-phase 5, ce miroir est une PASSE DÉCLARÉE (`demo.mirror`, ordre -10) de la frame orchestrée.

## L'orchestration des passes (consolidation, sous-phase 5)

La frame est un PLAN EXPLICITE de passes déclarées — plus une succession dispersée d'appels. Chaque passe possède sa destination (surface ou cible), son traitement d'entrée, sa caméra et SA file de draws ; le renderer les exécute dans un ordre déterministe et en rend compte. C'est l'accueil des ombres, du transparent, du debug rendering et du post-process — sans render graph universel.

**Le vocabulaire** (`pass.rs`, jamais un type backend) :

```rust
let mirror = renderer.add_pass(
    &RenderPassDescriptor::new("app.mirror", RenderDestination::Target(target))
        .with_load(PassLoad::Clear(color))     // ou Keep : conserver la couleur
        .with_camera(view_projection)
        .with_reads(&[autre_cible])            // les lectures DÉCLARÉES
        .with_order(-10),                      // négatif = avant la principale
)?;
renderer.queue_draw_to(mirror, command)?;      // la file de la passe
renderer.set_pass_camera(mirror, vp)?;         // réglages par frame
renderer.set_pass_enabled(mirror, false)?;     // sautée proprement
renderer.update_pass(mirror, &descriptor)?;    // re-déclaration (après un resize de cible)
```

- **La passe principale `chaos.main`** (surface, ordre 0) existe à la construction — `queue_draw`, `set_clear_color` et `set_view_projection` la pilotent, le comportement historique est inchangé. `main_pass()` donne son handle ; la désactiver est le mécanisme officiel du rendu tout-hors-écran. Sa destination et son label sont protégés.
- **L'ordre est DÉTERMINISTE** : tri stable par `order` croissant, égalités départagées par l'ordre d'enregistrement — jamais un tri topologique (le render graph avancé pourra remplacer l'ordonnancement explicite plus tard, sans changer le contrat backend).
- **Les dépendances invalides sont REFUSÉES à la déclaration**, en nommant la règle : label vide/dupliqué/préfixe `chaos.` réservé ; destination ou lecture périmée ; lecture de sa propre destination (feedback) ; et l'invariant d'ordonnancement — si une passe ÉCRIT une cible qu'une autre LIT, l'écrivaine doit précéder la lectrice (« schedule it earlier »). Une lecture sans écrivain la même frame reste légale (contenu d'une frame précédente). L'invariant est déclaratif (indifférent à `enabled`), revalidé sur tout le registre à chaque `add_pass`/`update_pass` — un refus laisse tout intact.
- **Au frame-time, jamais fatal, tout observable** : passe désactivée → sautée ; cible périmée (resize/destroy après coup) → la passe s'AUTO-DÉSACTIVE avec un warn unique (pas de spam — `update_pass` avec le handle frais la rebranche) ; un draw dont le material échantillonne la destination de SA passe (feedback non déclaré) → écarté avec warn, comme un draw périmé.
- **Le backend exécute** : acquisition de surface au plus UNE fois (avant sa première passe surface ; indisponible → passes surface sautées, passes cible exécutées), un submit PAR PASSE (les uniforms — buffer frame, slots objets — sont partagés entre passes : la soumission par passe est LE contrat de correction, la timeline de queue garantissant l'ordre écriture→commandes ; dynamic offsets = l'optimisation notée), présentation au plus UNE fois après la dernière passe.
- **La profondeur suit deux règles symétriques** (backend) : stockée SEULEMENT si une passe ultérieure de la même destination arrive en `Keep` (sinon Discard — l'optimisation tile-based conservée) ; chargée SEULEMENT si `Keep` et qu'une passe antérieure de la même destination l'a produite cette frame — sinon effacée à 1.0, même en `Keep` : `Keep` conserve la COULEUR, jamais un contenu de profondeur indéfini.
- **Les diagnostics** : `frame_report()` — passe par passe dans l'ordre d'exécution : label, destination, draws résolus, issue (`Executed`, `Disabled`, `StaleTarget`, `SurfaceSkipped`). C'est une reconstruction du renderer (le backend rend UNE issue de présentation) ; vide avant la première frame, intact après `render_to_target`. Les metrics moteur comptent désormais les draws de TOUTES les passes.
- **Limites V1 assumées** : passes permanentes (pas de suppression — la désactivation en tient lieu, handle simple non générationnel), une caméra et une destination par passe, pas de MRT, `reads` déclaratif (non recoupé avec les textures réelles des materials — le feedback effectif est, lui, écarté au frame-time).

## L'éclairage (Lighting V1 — consolidation, sous-phase 7)

Les lumières sont des DONNÉES moteur pures (`light.rs` — jamais un type wgpu), soumises par frame comme les draws, et les materials `Lit` y réagissent réellement : `sample × base_color × (ambiante + Lambert diffus)`.

**Le vocabulaire** : `Light::{Directional, Point, Spot}` — couleur linéaire, intensité, portée (ponctuelle/spot), orientation (directionnelle/spot), cône (`inner_angle` < `outer_angle` STRICT), `enabled` (une lumière désactivée reste soumissible, elle est écartée de la frame — le toggle sans re-plomberie).

```rust
renderer.set_ambient_light(Color::WHITE, 0.08);          // RÉGLAGE persistant (patron clear_color)
renderer.submit_light(Light::directional(dir, color, 0.9));   // par frame, comme queue_draw
renderer.submit_light(Light::point(pos, color, 2.5, 5.0));    // vidées par clear_draws
```

- **La sélection est DÉFINIE et prévisible** : au rendu, la collection filtre les désactivées, normalise les directions, et tronque à `MAX_LIGHTS` (16) en ordre de soumission — les premières gagnent, un warn PAR ÉPISODE de dépassement (armé au premier, réarmé sous la limite — jamais de spam). Une lumière INVALIDE (direction nulle, intensité négative, cône dégénéré, NaN) est écartée AU SUBMIT avec un warn — jamais envoyée au GPU.
- **Le transport** : `FramePlan.lights` (la vue structurée — le renderer ne dépend ni de Scene ni de l'ECS) → un uniform buffer de 1 056 octets au groupe(0) binding(1) (`inputs::FRAME_LIGHTS_BINDING`) : ambiante (16 o) + compte (16 o) + 16 entrées de 64 octets (position+portée / direction+genre / couleur+intensité / cosinus du cône). Écrit UNE fois par plan — l'éclairage est constant sur toutes les passes. Les shaders qui ne déclarent pas ce binding restent valides (la règle WebGPU) : `chaos.vertex_color` et `chaos.textured` n'ont pas bougé.
- **Les normales** : `LitVertex { position, normal, uv }` (stride 32), la matrice des normales (`chaos_core::math::normal_matrix`, inverse-transposée — l'échelle non uniforme préservée, singulier → identité) voyage dans `FrameDraw.normal` et les uniforms d'objet (64 → 128 octets). Le shader renormalise au fragment (l'interpolation et l'échelle uniforme changent la longueur).
- **Le modèle d'illumination V1** : ambiante + Lambert diffus ; atténuation ponctuelle `clamp(1-(d/range)²)²` ; spot en fondu `smoothstep` entre les cosinus du cône. L'ambiante par défaut est (noir, 0) — sans ambiante ni lumière, une surface `Lit` est NOIRE (documenté, voulu : l'éclairage est explicite).
- **Limites V1 EXPLICITES** : pas de spéculaire (il viendra avec la position caméra du PBR), éclairage GLOBAL par frame (mêmes lumières pour toutes les passes — miroir compris), pas de culling de lumières par objet (les 16 s'appliquent à tous les draws lit), pas de flip de normale backface (un double-sided éclairé par-derrière reçoit la normale avant). Les ombres directionnelles ont atterri avec la sous-phase 10 (section « Les ombres »).
- **La preuve vivante (démo)** : sol et cubes de la scène en `Lit` (les normales du sol SYNTHÉTISÉES — `floor.glb` n'en porte pas), une directionnelle chaude togglée par K, trois ponctuelles colorées orbitantes suivies de leurs marqueurs, un spot cyan sur le cube central.

## Le matériau PBR (V1 — consolidation, sous-phase 8)

Le modèle `MaterialModel::Pbr` rend les surfaces PHYSIQUEMENT PLAUSIBLES (Cook-Torrance GGX, workflow metallic/roughness, forward) sous les lumières V1 — chaque propriété en CONSTANTE et/ou TEXTURE. **LE CONTRAT, documenté** :

| Propriété | Constante (défaut) | Texture (fallback neutre) | Espace de couleur |
|---|---|---|---|
| Base color | `base_color` (blanc) | slot 0 (`chaos.white`) | **sRGB** |
| Metallic | `metallic` (0 — diélectrique) | canal **B** du slot 3 (`chaos.white`) | linéaire |
| Roughness | `roughness` (1 — mat) | canal **G** du slot 3 (`chaos.white`) | linéaire |
| Normal map | — | slot 4 (`chaos.normal_flat`) | linéaire, tangent-space, **+Y vert** |
| Occlusion | — | canal **R** du slot 5 (`chaos.white`) | linéaire |
| Émissif | `emissive` (noir — éteint) | slot 6 (`chaos.white`) | **sRGB** |

- **Conventions glTF** : le packing metallic/roughness (B/G), l'AO en canal R, la normal map +Y vert, la règle constante × texture — les assets glTF s'y coucheront tels quels le jour où les décodeurs d'images arriveront (l'IMPORT des materials glTF attend PNG/JPEG — hors périmètre, conventions déjà alignées). Un material nu = un diélectrique mat.
- **Le traitement des normales** : le repère cotangent est DÉRIVÉ à l'écran (`dpdx`/`dpdy` — aucun attribut tangente sur `LitVertex`) ; artefacts assumés V1 : facettes sous normal map forte sur surfaces courbes, discontinuités d'un pixel aux seams UV ; des UV dégénérés retombent sur la normale géométrique (jamais de NaN) ; les tangentes de vertex (MikkTSpace) viendront avec leur besoin. `double_sided` retourne la normale au fragment (`front_facing`).
- **La BRDF** : diffus Lambert pondéré `(1-F)(1-metallic)`, spéculaire GGX + Smith-Schlick (variante direct-lighting, k=(r+1)²/8) + Fresnel-Schlick (F0 = mix(0.04, albedo, metallic)) ; roughness clampée ≥ 0.045 ; ambiante plate × albedo × AO, PLUS la contribution environnementale (IBL — section « L'environnement et le ciel ») ; l'émissif s'ajoute en sortie.
- **La relation Material System ↔ shader** : `Pbr` → `chaos.pbr` par le cache de permutations, comme tout modèle ; un `Custom` à entrées material voit TOUT le groupe(2) (les 7 slots) — lire les propriétés PBR est la responsabilité de son shader. Les propriétés PBR sur `Unlit`/`Lit` sont REFUSÉES en nommant la propriété (jamais inertes en silence).
- **La mise à jour contrôlée** : `set_material_metallic`/`set_material_roughness`/`set_material_emissive` écrivent le buffer 48 o EN PLACE (le chemin des pulsations et du tuning éditeur) ; les 4 slots de textures PBR sont FIGÉS à la création (recréer le material — l'asymétrie documentée), la texture de base et le sampler restent mutables.
- **Limites V1 EXPLICITES** : forward ; tone mapping Reinhard PAR MATERIAL dans le shader (provisoire — le post-process par frame le remplacera : un objet Pbr et un objet Lit voisins ne compressent pas pareil) ; UN sampler pour toutes les textures du material ; alpha STRAIGHT sans réfraction ; le spéculaire des vignettes `render_to_target` est ancré à l'origine (pas de position caméra sur ce chemin) ; l'IBL est arrivée avec l'environnement (sous-phase 9 — ses propres approximations documentées) ; pas de clearcoat/sheen/transmission.
- **La preuve vivante (démo)** : la grille 4×4 de sphères (metallic → droite, roughness → bas — le gradient des highlights), le cube normal-mappé (map procédurale), la sphère émissive pulsante.

## L'environnement et le ciel (V1 — consolidation, sous-phase 9)

La scène gagne un environnement VISUEL et LUMINEUX : une cubemap (HDR recommandé — `Rgba16Float`) qui se dessine en CIEL de fond et contribue au PBR en IBL — les métaux reflètent le monde au lieu du noir.

**L'API** — un réglage persistant (le patron de l'ambiante), jamais vidé par `clear_draws` :

- `set_environment(&EnvironmentDescriptor { cubemap, intensity (1.0), sky (true) })` : la cubemap doit être vivante et de kind `Cube` (refus nommé sinon), l'intensité finie et positive ou nulle. Re-poser le MÊME cubemap = mise à jour intensité/ciel SANS rebind backend ; un autre cubemap rebinde le groupe frame. `clear_environment()` (idempotent) rebinde le cube fallback noir interne. `environment_info()` → label, intensité, ciel, niveaux de mips (l'inspection éditeur).
- `set_exposure(f32)` / `exposure()` : l'EXPOSITION globale, appliquée avant le tone mapping — les chemins tone-mappés seulement (PBR et ciel ; `Unlit`/`Lit` ne la lisent pas, documenté). Refus d'une valeur non finie ou non strictement positive.
- **Protection** : détruire la cubemap de l'environnement ACTIF est refusé (« clear it first ») — pas d'état périmé possible.

**Le ciel** (`chaos.sky`) : un triangle plein écran généré par le shader (aucune géométrie), dessiné à la profondeur MAXIMALE exacte (z = w) sous le test `LessEqual` — il ne couvre que les pixels laissés au clear. La direction de vue vient de la DÉPROJECTION near → far par la vue-projection INVERSE de la passe : indépendante de `camera_position`, correcte pour toute caméra (les vignettes `render_to_target` comprises). Le renderer INJECTE le draw ciel entre les opaques (fill-rate) et les transparents (qui se mélangent par-dessus) de chaque passe **`Clear`** — une passe `Keep` ne le reçoit JAMAIS (il repeindrait l'image conservée sous une profondeur repartie à 1.0, ou le fond d'une caméra étrangère). Le draw injecté compte dans `PassReport.draws` (c'est un draw réel soumis) ; `draw_count()` ne compte que les soumis — l'asymétrie documentée. Le pipeline ciel a son propre cache de permutations par format de destination (`chaos.sky[.{Format}]`) ; un échec de création est mémoïsé avec un warn unique et le ciel abandonné — jamais la frame.

**L'IBL V1 dans `chaos.pbr`** — des approximations ASSUMÉES, documentées :

- **Spéculaire** : `textureSampleLevel(env, reflect(-V, N), roughness × max_lod)` × la **BRDF d'environnement analytique de Karis** — les mips de la cubemap tiennent lieu de préfiltre (box, pas GGX : les reflets rugueux sont approximatifs) ; pas de BRDF LUT en V1.
- **Diffuse** : le DERNIER mip (1×1×6 — six directions interpolées) tient lieu d'irradiance, × albedo × (1 − metallic).
- Les deux × intensité × AO, AJOUTÉS à l'ambiante plate (qui reste — prévisible, sans branche shader : le cube fallback noir annule la contribution quand rien n'est configuré). Puis `couleur × exposition` avant le Reinhard.
- Le préfiltre GGX, la BRDF LUT et les harmoniques sphériques viendront avec leurs besoins ; les coutures inter-faces des mips par-face se voient aux LODs élevés (limite V1).

**La mécanique backend** (wgpu confiné) : le groupe(0) porte la cubemap (binding 2, `texture_cube`) et son sampler interne (binding 3, linéaire trilinéaire clamp) dès la construction — un cube fallback noir 1×1×6 (zéro-initialisé, organe interne comme la profondeur) est bindé tant que rien n'est configuré. `GraphicsBackend::set_environment(Option<TextureHandle>)` crée la vue Cube et RECONSTRUIT le bind group frame (les soumissions en vol survivent — garantie wgpu). `FrameUniforms` passe à 160 octets (vue-projection, position caméra, vue-projection INVERSE, paramètres d'environnement) ; `FramePlan.environment` transporte intensité et exposition. Une vue-projection singulière produit une inverse non finie → directions de ciel NaN (visible seulement environnement actif, documenté).

**La preuve vivante (démo)** : la cubemap HDR procédurale `demo.sky` (gradient + disque solaire à 12.0, mips générées), la colonne métallique de la grille PBR qui REFLÈTE le ciel, le miroir qui le montre aussi ; **E** bascule l'environnement (retour fond uni + ambiante plate), **V**/**B** exposent ciel et PBR ensemble.

## Les ombres (Shadows V1 — consolidation, sous-phase 10)

La lumière directionnelle principale PROJETTE des ombres temps réel : une shadow map depth-only rendue par une passe DÉRIVÉE en tête de chaque plan, échantillonnée par les shaders éclairés (`chaos.lit`, `chaos.pbr`) au sampler de comparaison (PCF 3×3 matériel). Tout est BACKEND-INTERNE, conformément à l'invariant de la carte d'atterrissage — aucun format profondeur public, aucun sampler de comparaison public : le précédent de la profondeur (V2.6), appliqué aux ombres.

**L'API** — un réglage persistant (le patron de l'environnement), jamais vidé par `clear_draws` :

```rust
renderer.set_directional_shadow(&DirectionalShadowDescriptor::new(
    ShadowVolume::new(center, half_extents))       // le volume monde EXPLICITE
        .with_resolution(2048)                     // 16..=8192 (défaut 2048)
        .with_depth_bias(0.002)                    // unités light-clip, à chaud
        .with_normal_bias(0.02))?;                 // unités monde, à chaud
renderer.clear_directional_shadow()?;              // libère la map (idempotent)
renderer.directional_shadow_info();                // l'inspection éditeur
```

- **Le volume est EXPLICITE et indépendant de la caméra** (`ShadowVolume { center, half_extents }` — x/y latéraux, z le long des rayons, dans le repère de la lumière) : la stabilité sous mouvements de caméra est acquise PAR CONSTRUCTION. La vue de lumière (`shadow::light_view_projection`, fonction pure testée) est une orthographique 0..1 cadrée sur le volume, up de secours quand la direction est quasi verticale. Le fitting caméra, le snapping de texels et les cascades sont les extensions notées.
- **La lumière qui projette est la PREMIÈRE directionnelle activée et valide de la collection** (« les premières gagnent », la règle de la troncature — l'index voyage aux shaders, jamais hors du tableau GPU). Réglages posés sans directionnelle soumise → pas de passe d'ombre, facteur 1 partout, rien de fatal — le toggle du soleil (K dans la démo) emporte les ombres avec lui.
- **Cast/receive sont des ÉTATS DE RENDU DU MATERIAL** (comme `double_sided`/`opacity`, figés à la création) : `without_shadow_cast()` retire des projeteurs, `without_shadow_receive()` des receveurs — refusé en nommant la règle sur un modèle qui ne réagit pas à l'éclairage (`VertexColor`, `Unlit`, `Custom` sans entrées). Un TRANSPARENT ne projette jamais en V1 (documenté) ; `receive` voyage dans `MaterialUniforms.params.z` (48 octets inchangés).
- **Les casters sont dérivés au resolve** : l'union des draws `cast_shadows` dont la catégorie PROJETTE (`MaterialOpacity::casts_shadows()` — Opaque et Masked, ce dernier en silhouette pleine V1) de toutes les passes actives (duplicatas multi-passes acceptés — la profondeur est idempotente, la dédup est l'optimisation notée), collectés à la résolution des materials — le ciel injecté et les transparents ne peuvent pas y fuir. Chaque caster reçoit la permutation d'ombre de son (vertex layout, culling) : cache dédié (le patron du ciel), labels `chaos.shadow.{stride}[.double_sided]`, échec ou layout sans position `Float32x3@0` mémoïsé avec un warn unique. Depuis la sous-phase 13, la moisson est CULLÉE par le frustum de la LUMIÈRE — jamais celui d'une passe (section « La visibilité »).
- **Le transport** : `FramePlan.shadow: Option<FrameShadowPass>` (vue de lumière, résolution, biais, index de lumière, casters) ; la queue OMBRE de `LightsUniforms` (1 056 → 1 136 octets : matrice @1056, paramètres @1120) écrite une fois par plan — toutes les passes éclairées (miroir compris) échantillonnent la MÊME map. `GraphicsBackend::set_shadow(Option<ShadowConfig>)` gère la ressource : re-poser la même résolution = zéro appel backend (volume et biais sont des données par frame — le tuning à chaud sans recréation), une autre résolution recrée la map proprement.
- **La mécanique backend** (wgpu confiné) : map `Depth32Float` échantillonnable + sampler de comparaison LessEqual/Linear (le PCF matériel) aux bindings 4–5 du groupe frame ; un fallback 1×1 effacé à 1.0 est bindé sans réglages — « tout éclairé » sans branche shader (le patron du cube d'environnement noir ; `write_texture` est INTERDIT sur Depth32Float et wgpu zéro-initialise à 0.0 = tout ombré : l'initialisation passe par une passe de clear dédiée). La passe d'ombre s'exécute EN TÊTE du plan (uniforms → un submit, le contrat inter-passes), zéro attachement couleur, `Clear(1.0)`/`Store` — jamais dérivée de `depth_operations`. Son pipeline vise un groupe(0) RÉDUIT (buffer frame seul) : binder le groupe complet serait un conflit d'usage wgpu (la map y est texture ET attachement). Les vues vives (environnement, ombre) sont RETENUES côté uniforms — rebinder l'une ne perd jamais l'autre.
- **Le shading** : le facteur d'ombre (PCF 3×3 via `textureSampleCompareLevel` — aucune contrainte d'uniformité) n'atténue QUE la contribution directe de la lumière projetante — l'ambiante et l'IBL restent (physiquement correct, la scène reste lisible). Biais de normale en unités monde AVANT projection (la normale GÉOMÉTRIQUE en PBR — la normal map ne pousse pas le point dans la surface), biais de profondeur soustrait à la référence, hors volume → facteur 1.
- **Diagnostics** : `frame_report().shadow` (`ShadowReport { draws, draw_calls, culled, resolution }` — un rapport DÉDIÉ, `RenderDestination` n'est pas pollué), `directional_shadow_info()`, la famille `shadow_maps` des stats (résolution² × 4 octets, retour à zéro au clear). Les draws d'ombre sont dérivés : comptés au rapport, jamais dans `draw_count()` (la règle du ciel injecté).
- **Limites V1 EXPLICITES** : UNE directionnelle projette (ponctuelles/spots — cube maps ou atlas — avec leur besoin ; `ShadowConfig` est le point d'extension), volume statique explicite (pas de fitting caméra ni cascades), PCF 3×3 fixe, pas de bias rasterizer slope-scaled (le biais est à l'échantillonnage — réglable à chaud), pas de casters alpha-masqués, `render_to_target` ne rend pas de passe d'ombre (ses draws échantillonnent la map du dernier plan), le culling des casters suit le material (le front-face culling anti-acné est l'optimisation notée).
- **La preuve vivante (démo)** : le soleil projette la scène entière sur le sol (volume explicite couvrant sol, ronde et grille PBR — 2048 texels), la sphère émissive est `without_shadow_cast` (la preuve par l'absence), **N** bascule les ombres (libération/recréation propres), **K** coupe le soleil — les ombres disparaissent avec lui.

## L'opacité et l'ordre de rendu (sous-phase 11)

`MaterialOpacity` est l'AUTORITÉ UNIQUE des contrats de rendu par catégorie — la permutation de pipeline, la partition de la passe et la collecte des casters d'ombre consomment ses méthodes (`writes_depth`, `blends`, `casts_shadows`, l'entrée fragment, le suffixe de label), jamais des règles locales. Trois catégories, alignées sur les `alphaMode` de glTF :

| Contrat | `Opaque` | `Masked` (alpha cutout) | `Transparent` |
|---|---|---|---|
| Test de profondeur | oui (`Less`) | oui (`Less`) | oui (`Less`) |
| Écriture de profondeur | oui | oui | NON (lecture seule) |
| Blend | REPLACE | REPLACE | ALPHA |
| Entrée fragment | `fs_main` | `fs_masked` (discard sous `alpha_cutoff`) | `fs_main` |
| Projette des ombres | oui | oui — silhouette PLEINE V1, trous compris | jamais |
| Reçoit des ombres | oui (si `receive_shadows`) | oui (si `receive_shadows`) | oui (si `receive_shadows`) |
| Ordre dans la passe | premier, groupé par material | deuxième, groupé par material | dernier, TRIÉ arrière → avant |

- **`Masked`** : le fragment dont l'alpha (sample × base_color) passe sous `alpha_cutoff` (défaut 0.5 — glTF) est ÉLIMINÉ ; la profondeur s'écrit comme un opaque — pas de blending, pas de tri. Le cutoff est une constante material (`with_alpha_cutoff`, canal `params.w` — le vec4 de paramètres est COMPLET : metallic, roughness, receive_shadows, alpha_cutoff), modifiable à chaud par `set_material_alpha_cutoff` (refusé hors Masked, bornes 0..=1 nommées). Masked est REFUSÉ sur un modèle sans entrées material (aucun alpha à tester) ; un `Custom` masked doit exposer `fs_masked` (la délégation, patron de `pbr_inputs`) ; un cutoff hors défaut sur une autre opacité est refusé en nommant la propriété.
- **L'entrée `fs_masked`** : les fragments de `chaos.textured`/`chaos.lit`/`chaos.pbr` sont une fonction partagée `shade(...)` sous DEUX points d'entrée minces — `fs_main` (sans `discard` : l'early-Z des pipelines opaques est préservé) et `fs_masked` (discard APRÈS `shade()` — échantillonnages et dérivées sous contrôle uniforme —, sortie opaque alpha 1). La permutation masked n'est qu'un `fragment_entry` différent (`PipelineDescriptor::with_fragment_entry`) — zéro état backend de plus, le label porte `.masked`.
- **L'ordre à QUATRE TEMPS de chaque passe** : opaques → masked (tous deux écrivent la profondeur ; les opaques d'abord — l'early-Z aide les masked) → ciel (passes `Clear` seulement, inchangé) → transparents triés. Le debug rendering (sous-phase 14) y a PRIS place : ses batches s'encodent APRÈS les transparents, l'overlay en dernier (section « Le debug rendering »).
- **Le tri des transparents** : ARRIÈRE → AVANT par distance² à la caméra de SA passe (la translation du modèle comme proxy de l'objet), comparaison `total_cmp` (jamais un NaN qui panique), tri STABLE — à distance égale, l'ordre de soumission gagne (les égalités restent regroupées par material). Le tri suit la caméra frame après frame. Le regroupement par material est SACRIFIÉ dans cette classe (la correction avant le batching — les opaques gardent le leur) ; `render_to_target` trie depuis l'origine (sa limite caméra existante).
- **Diagnostics** : chaque `PassReport` porte sa VENTILATION (`DrawBreakdown { opaque, masked, transparent, injected }` — injected = le ciel, demain le debug) ; `draws` reste le total.
- **Limites V1 EXPLICITES** : tri par OBJET (transparents larges ou sécants peuvent se tromper — le tri par triangle, le depth peeling et l'OIT sont les extensions notées), silhouette d'ombre PLEINE des masked (les casters alpha-testés — extension notée), pas de pré-passe de profondeur, pas de dithered/hashed alpha.
- **La preuve vivante (démo)** : la GRILLE masked (texture procédurale à pastilles transparentes, quad `Lit` double-sided près de la ronde — les trous nets à l'écran, l'ombre au sol en silhouette pleine : l'artefact V1 visible et assumé) et le TRIO de verres étagés en profondeur (le panneau pulsant + un rouge + un vert) — quel que soit l'angle de survol caméra, les panneaux se mélangent dans le bon ordre.

## L'instancing (V1 — sous-phase 12)

Le renderer REGROUPE SEUL les draws compatibles en draws INSTANCIÉS — les consommateurs soumettent toujours objet par objet (`DrawCommand`), aucune API de batch n'existe : la décision appartient au renderer.

- **La compatibilité** : même **(material, mesh)** — donc même pipeline, même binding, mêmes buffers —, même passe et même catégorie d'opacité par construction (résolution par passe, classes séparées). La `RenderQueue` trie par la clé composite `(material, mesh)` : les compatibles deviennent des RUNS consécutifs, fusionnés À PARTIR DE 2 (un run de 1 reste un draw classique). Les TRANSPARENTS ne sont JAMAIS instanciés en V1 — leur tri par profondeur individuel prime.
- **Les données par instance** : matrice modèle + matrice des normales (128 octets — le miroir des `ObjectUniforms`), dans un SECOND slot de vertex buffer à cadence Instance (`instance_transforms_layout()` — l'autorité : huit `Float32x4` aux locations 4..=11, verrouillée par test). L'instance buffer backend est croissant et PARTAGÉ entre les passes (le contrat write → submit par passe, comme le buffer frame et les slots objets).
- **Les permutations INSTANCIÉES** : entrée vertex `vs_instanced` (les cinq shaders à géométrie l'exposent — verrou naga), cache dédié à valeurs `Option` — un échec de création (ex. un `Custom` sans `vs_instanced`) est MÉMOÏSÉ avec un warn unique et le run reste en draws classiques, jamais la frame ; un `Custom` opte à l'instancing en exposant `vs_instanced` (la délégation documentée). Labels `…​.instanced`, format de cible compris (le descripteur VISE le format de la destination, pas seulement le label).
- **Les ombres profitent pareil** : la moisson des casters (l'union des passes actives) est TRIÉE par clé puis fusionnée — les duplicatas multi-passes deviennent UN run (une passe de profondeur est indifférente à l'ordre) ; permutations `chaos.shadow.{stride}[.double_sided].instanced`.
- **Les diagnostics prouvent le bénéfice** : `PassReport.draws` reste les OBJETS logiques (la sémantique historique), `PassReport.draw_calls` dit les SOUMISSIONS réelles (≤ draws) — `ShadowReport` pareil ; le journal mock suffixe ` inst=N`. `draw_count()` et les metrics moteur restent les soumissions logiques du consommateur.
- **Le coût CPU chute mécaniquement** : les slots d'uniforms d'objets suivent les draw calls, plus les objets (la démo : 1 241 objets en passe principale, 32 slots — contre 45 slots pour 41 objets avant l'essaim).
- **Limites V1 EXPLICITES** : pas de storage buffers d'instances (les foules ≫ 10⁵), pas de culling par instance à l'INTÉRIEUR d'un batch formé — le culling par draw se joue AVANT la fusion (sous-phase 13, section « La visibilité »), les transparents hors batching (le tri par instance est l'extension notée), le slot objet des draws instanciés est écrit mais non lu (l'alignement d'index draw↔slot est le contrat — le skip est l'optimisation notée), pas d'API de batch consommateur, pas d'indirect/GPU-driven.
- **La preuve vivante (démo)** : l'ESSAIM — 1 200 mini-cubes `Lit` en double hélice animée au-dessus du centre, transforms recalculés CHAQUE frame, soumis un par un → UN draw instancié en passe principale et UN caster instancié dans l'ombre ; et la RONDE historique (8 cubes, un mesh, un material) fusionne d'elle-même — pas une ligne du consommateur n'a changé. **O** log le rapport draws → draw calls par passe.

## La visibilité (bounds & frustum culling V1 — sous-phase 13)

Le renderer ne paie plus les objets hors champ : chaque draw est testé contre le frustum de SA vue au resolve — rejeté, il n'est jamais résolu, jamais compté aux slots, jamais soumis. Le culling est un service INTERNE du resolve : aucun chemin public de draw ne change, le consommateur soumet toujours objet par objet.

- **Les bounds locaux vivent dans le `MeshRecord`** (`bounds: Option<Aabb>`) — l'atterrissage annoncé depuis V1. Calculés AUTOMATIQUEMENT à la création depuis les positions (les trois `create_*_mesh` — le chemin assets en hérite sans couture) ; `None` (géométrie vide, positions non finies — warn à la création) = JAMAIS cullé, le défaut sûr. Inspection : `mesh_bounds(handle)`. L'`Aabb` vit dans `chaos_core::math` (le vocabulaire servira la physique demain) : `from_points` refuse les bounds invalides à la source (`Option` — vide ou non fini n'existe jamais), `transformed(Mat4)` est la méthode d'Arvo — conservatif sous rotation. Les bounds MONDE se calculent UNE fois par draw au resolve, partagés entre le test caméra et le test lumière.
- **Un frustum PAR VUE** (`visibility.rs`, public — l'outil du futur éditeur) : `Frustum::from_view_projection` (extraction de Gribb-Hartmann, profondeur 0..1 — nos conventions) + `intersects(&Aabb)` (test du p-vertex sur le signe seul — pas de normalisation nécessaire, frontière INCLUSIVE). Chaque passe couleur construit le sien depuis SA `view_projection` — la principale, le miroir, `render_to_target` avec la VP qu'on lui donne : le frustum principal n'est JAMAIS appliqué aveuglément aux autres vues. La caméra par DÉFAUT (identité) ne voit que le cube NDC — un consommateur déclare sa caméra (la démo, à chaque update).
- **La moisson d'ombre est DÉCORRÉLÉE** : l'éligibilité caster teste le frustum de la LUMIÈRE (l'ortho du volume explicite), jamais celui d'une passe — un caster sorti de l'écran projette encore son ombre visible (l'anti-pop : LE piège classique du culling naïf, évité par construction et verrouillé par test) ; l'inverse aussi : un objet visible hors du volume ne projette plus.
- **CONSERVATIF par construction** : AABB monde d'Arvo + p-vertex inclusif — un objet partiellement visible n'est JAMAIS rejeté ; le sur-dessin des coins de frustum est le prix V1, assumé.
- **La coopération** : les transparents sont cullés AVANT le tri arrière → avant ; les instances AVANT la fusion — un run instancié ne contient QUE des visibles ; le ciel injecté n'est jamais concerné.
- **« Forcé visible » est un état de rendu du MATERIAL** (le patron cast/receive) : `frustum_culled` (défaut true), `without_frustum_culling()`, reflété par `MaterialInfo` — un tel draw saute les DEUX tests (passe et lumière). Aucun refus de modèle, aucune donnée GPU.
- **Les stats démontrent** : `PassReport.culled` (les objets rejetés par le frustum de LA passe) et `ShadowReport.culled` (les tentatives de moisson rejetées par le frustum de lumière — les duplicatas multi-passes comptent comme les draws, documenté). La touche **O** de la démo ajoute ` (N culled)` à ses lignes.
- **Limites V1 EXPLICITES** : un AABB par draw — pas de hiérarchie spatiale (BVH/octree), pas d'occlusion culling (explicitement hors périmètre), pas de culling par instance dans un batch formé (le culling précède la fusion), pas de bounds fins par sous-mesh, pas de culling des lumières, pas de LOD.
- **La preuve vivante (démo)** : voler hors de la scène — **O** montre les `culled` par passe qui évoluent, rien ne disparaît à tort aux bords de l'écran (le conservatisme à l'œil), les ombres des objets hors champ RESTENT au sol, le miroir cull avec SA caméra fixe.

## Le debug rendering (V1 — sous-phase 14)

Le langage visuel COMMUN des données spatiales : les futurs systèmes (physique, IA, éditeur) et le contenu SOUMETTENT des primitives (`Renderer::queue_debug`), le renderer les dessine — sans dépendre de l'UI, de l'éditeur, de la physique, de l'ECS ni des scènes. C'est un service du renderer, comme les lumières et l'environnement.

```rust
renderer.queue_debug(DebugDraw::aabb(bounds)         // ou line/ray/arrow/point/marker/
    .with_color(Color::rgb(1.0, 0.85, 0.2))          //    axes/grid/sphere/frustum/light
    .with_duration(2.0)                              // 0 (défaut) = la frame ; > 0 = retenue
    .with_category("physics")                        // le levier d'activation par famille
    .overlay()                                       // par-dessus tout (sinon testé)
    .for_pass(handle));                              // la principale par défaut
renderer.advance_debug_time(delta);                  // les retenues expirent seules
renderer.set_debug_enabled(false);                   // le toggle global (défaut : actif)
renderer.set_debug_category_enabled("physics", false);
```

- **Le vocabulaire (`debug.rs`)** : onze formes — Line, Ray (portée + croix au bout), Arrow, Point (croix), Marker (octaèdre filaire — distinct du point), Axes (XYZ = RVB canoniques, la couleur du draw ignorée), Grid (plan XZ), Aabb (les 12 arêtes), Sphere (trois grands cercles), Frustum (les 8 coins dé-projetés d'une vue-projection — profondeur 0..1) et Light (la DONNÉE dessinée : flèches pour une directionnelle depuis une ancre, sphère de portée pour une ponctuelle, cône inner/outer pour un spot — la couleur DE la lumière par défaut). La TESSELLATION est PURE et publique (`DebugShape::tessellate` — l'autorité géométrique, testée sans GPU) : tout devient des SEGMENTS monde (`DebugVertex` : position + couleur RGBA, stride 28).
- **La validation AU SUBMIT** (le patron `submit_light`) : géométrie non finie, taille/pas/rayon non positifs, durée négative, vue-projection non inversible, catégorie vide, passe inconnue → warn + écarté, jamais stocké, jamais au GPU.
- **Deux durées de vie** : `duration == 0` (défaut) = la frame de SIMULATION — vidée par `clear_draws` comme les draws (les re-présentations du resize re-présentent le même debug) ; `> 0` = RETENUE — survit à `clear_draws`, décomptée par `advance_debug_time(delta)` (le renderer n'a PAS d'horloge : le consommateur fournit le temps), expire seule. Inspection : `debug_stats()`.
- **L'activation** : le toggle GLOBAL + les CATÉGORIES (`String` libre, `general` par défaut) — les filtres agissent au RENDU seulement : les retenues d'une catégorie désactivée continuent d'expirer et réapparaissent au réveil. L'activation par primitive individuelle passerait par un handle — hors V1, la catégorie en tient lieu.
- **Deux modes de PROFONDEUR, jamais d'écriture** : `Scene` (défaut) = testé `LessEqual` — la primitive est occludée par la scène ; `Overlay` = `DepthCompare::Always` (nouveau variant public) — dessiné par-dessus tout. Les deux blend en alpha (la transparence des pipelines : profondeur en lecture seule — exactement le contrat).
- **Le transport** : par passe, les primitives visibles sont tessellées en DEUX plages d'un même tableau (`FramePass.debug_vertices`) et au plus DEUX batches (`FrameDebugBatch { pipeline, first_vertex, vertex_count }`) — Scene puis Overlay, encodés APRÈS les transparents (le slot réservé depuis la sous-phase 11), l'overlay en DERNIER. Côté backend (wgpu confiné) : un buffer de sommets croissant partagé entre les passes (le patron de l'instance buffer, contrat write → submit), et un slot d'objet par batch écrit à l'identité (le layout standard [frame, objet] est conservé — le shader debug ne lit pas le groupe objet).
- **Les pipelines : le patron du ciel** — cache par (format de destination, mode de profondeur), labels `chaos.debug[.overlay][.{Format}]`, topologie `LineList`, échec mémoïsé avec un warn unique (le debug est abandonné, jamais la frame). Le shader `chaos.debug` est MINIMAL : la seule vue-projection au binding (0,0), position + couleur par sommet — ni `vs_instanced`, ni `fs_masked`, ni ombres (le debug n'est pas un material) ; verrouillé par test naga (entrées exactes, bindings, miroir de `DebugVertex::layout()`).
- **Les comptes** : chaque primitive DESSINÉE = un objet `injected` (`PassReport.draws` la compte — la règle du ciel) ; chaque batch = une soumission (`draw_calls` — la sémantique instancing). `draw_count()` ignore le debug (pas une soumission du consommateur). Le debug n'est PAS cullé par le frustum (on veut voir les bounds hors champ — assumé), `render_to_target` n'en dessine pas (le chemin immédiat, la règle de la passe d'ombre), une passe désactivée le saute avec elle.
- **Limites V1 EXPLICITES** : lignes 1 px (wgpu ne porte pas les lignes larges — l'épaisseur viendrait en quads orientés caméra), pas de texte/étiquettes 3D, pas de volumes pleins translucides, pas de handle par primitive, pas de debug dans la passe d'ombre.
- **La preuve vivante (démo)** : la grille du sol (Scene — occludée) et les axes du monde (OVERLAY — visibles à travers la scène) au spawn, **X** les bounds monde de la ronde (la matière du culling rendue visible), **F** les frustums de la caméra du miroir et du volume de lumière, **J** les lumières dessinées comme données, **T** un marqueur retenu 3 s à la caméra (l'expiration à l'œil), **G** le toggle global.

## Les diagnostics du renderer (V1 — sous-phase 15)

Le renderer EXPLIQUE sa frame : `Renderer::diagnostics()` rend UN snapshot (`RendererDiagnostics`) de ce que la dernière frame orchestrée a rendu, éliminé, possédé et coûté — reconstruit à chaque `render_frame` (la règle de `frame_report` ; `render_to_target` n'y touche pas), affichable en lignes de log lisibles (`Display`) : utilisable SANS UI.

- **Les compteurs sont EXACTS et dérivés des draws RÉSOLUS** — jamais des objets soumis : l'analyse itère les ~30 soumissions GPU d'une passe, pas les 1 200 objets — le coût de l'instrumentation est STRUCTURELLEMENT borné (~2 `Instant` par passe, zéro allocation par objet). `FrameTotals` : soumis, résolus, classiques/instanciés/instances, cullés, injectés, TRIANGLES (indices ÷ 3 × instances — le ciel compte 1), segments de debug (à part — des lignes), changements de PIPELINE et de MATERIAL (le MIROIR exact de la règle de déduplication du backend — `bound_pipeline`/`bound_material`, batches debug compris), passes exécutées/sautées. Par passe (`PassStats`) : le même détail + `resolve_cpu_ms` ; l'ombre (`ShadowStats`) pareil. Les GAINS de l'instancing et du culling se LISENT : draws ≫ draw_calls, culled en chiffres.
- **Les coûts CPU sont MESURÉS** (`CpuCost`) : la résolution (Σ passes + moisson d'ombre), l'appel backend, le `render_frame` entier — `std::time::Instant`, en millisecondes.
- **Le temps GPU est HONNÊTE** (`GpuTiming`) : `Measured { milliseconds }` vient de VRAIES timestamp queries — la feature `TIMESTAMP_QUERY` n'est demandée que si l'adaptateur l'offre, le span va de la PREMIÈRE passe exécutée (l'ombre comprise) à la DERNIÈRE, résolu vers un ring de 3 readbacks mappés en asynchrone (`PollType::Poll` — JAMAIS bloquant : ring saturé = mesure sautée). La valeur est celle de la dernière frame RÉSOLUE (quelques frames de latence — documenté). Sinon `Unavailable { reason }` — feature absente, rien encore résolu, backend sans mesure (le mock le DIT) : **aucune valeur n'est jamais inventée**, le signalement explicite est le contrat.
- **Les ressources** : la photo `ResourceStats` embarquée (comptes, octets exacts, retraites) ; **la surface** (CUMULATIF) : présentées, indisponibles (les erreurs), reconfigurées (les récupérations), aire nulle ; **les fallbacks actifs** : permutations en échec mémoïsé (ciel/ombre/instancié/debug) + textures/samplers builtin vivants ; **le budget CPU** : `set_cpu_budget(Option<ms>)` (`None` défaut = jamais de dépassement — le patron du moteur), dépassements cumulés + drapeau de la dernière frame.
- **Le contrat backend** : `GraphicsBackend::gpu_frame_time() -> GpuTiming` — un futur backend natif mesure ou le dit ; l'isolation wgpu est intacte (le trait rend un type Chaos).
- **Limites V1 EXPLICITES** : le span GPU couvre la frame ENTIÈRE (le par-passe est l'extension — le query set est dimensionné pour), latence de quelques frames sur la mesure GPU, pas d'historique/percentiles (le moteur les possède — la couture metrics viendra), budget CPU seul (pas de budget GPU).
- **La preuve vivante (démo)** : **O** log le snapshot complet — sur Metal (M4 Pro), `gpu: 1.95 ms` MESURÉ à côté du CPU (`resolve 0.23 ms + backend 1.00 ms`), les 27 449 triangles, les 15/32 switches, les 17,6 Mo suivis.

## La robustesse multiplateforme (V1 — sous-phase 16)

Les différences entre GPU et plateformes ne produisent AUCUN comportement implicite : chaque capacité est détectée, utilisée quand elle existe, remplacée par un fallback documenté ou coupée proprement sinon, refusée clairement quand la configuration est impossible — et la décision est EXPLIQUÉE. `Renderer::capabilities()` rend le rapport (`RendererCapabilities` : backend, adaptateur, limites, une décision par domaine), capturé à l'initialisation, affichable sans UI (`Display`) — la ligne compacte à l'attach dit les écarts, le rapport complet part en debug.

**La politique des limites** : le renderer demande au device les DÉFAUTS WebGPU (le plancher portable — textures 8192 px, buffers 256 Mio, alignement uniforme 256 o, anisotropie ×16), délibérément pas les maximums de l'adaptateur — la robustesse avant la capacité (l'élévation ciblée est l'extension notée). Ces limites sont FAIT RESPECTER côté Renderer, AVANT le backend : un dépassement est un refus Chaos qui nomme la valeur ET la limite, jamais une erreur de validation wgpu.

| Capacité | Détection | Usage / Fallback / Refus |
|---|---|---|
| Features GPU (timestamps) | `adapter.features()` | demandée si offerte ; sinon `Disabled` — le temps GPU est DIT indisponible (jamais inventé) |
| Limites de textures | limites ACCORDÉES au device | textures 2D, faces de cube et render targets refusées au-delà en nommant la limite |
| Formats HDR (Rgba16Float) | cœur WebGPU | `Active` — garanti Metal/DX12/Vulkan, la garantie est DITE au rapport |
| Cubemaps | cœur WebGPU | `Active` — dit au rapport |
| Mipmaps | générées CPU | `Active` — indépendantes du device par construction |
| Anisotropie | plafond ×16 (cœur WebGPU) | bornée par le descripteur (self-check) ET par la limite SOURCÉE du device — deux refus distincts |
| Comparison samplers & shadow maps | cœur WebGPU (Depth32Float + PCF) | `Active` ; la résolution d'ombre est bornée par min(8192 engine, limite device) — le message nomme LA borne qui refuse |
| Timestamp queries | feature optionnelle | section « Les diagnostics » — mesuré ou `unavailable (raison)` |
| Modes de présentation | surface caps détectés | `AutoVsync`/`AutoNoVsync` selon la config — les modes Auto de wgpu retombent toujours proprement (politique portée par wgpu, dite) ; les modes offerts sont listés au rapport |
| Formats de profondeur | Depth32Float | `Active` — le format portable de référence |
| Tailles/alignements de buffers | limites accordées | buffers publics ET buffers de meshes refusés au-delà (le chemin commun) ; l'alignement uniforme est rapporté, non consommé (slots dédiés — dynamic offsets notés) |
| Ressources liées | limites accordées | rapportées (bind groups, textures/samplers par étage) — le moteur consomme 3 groupes et 7 textures : sous le plancher par construction |
| Limites de passes | limites accordées | rapportées (attachements couleur) — le moteur en utilise 1 |
| Surface format | surface caps | sRGB préféré = `Active` ; sinon `Fallback { no sRGB offered }` — le premier format offert |
| Fallbacks | — | les permutations dégradées et les builtins vivants sont comptés aux diagnostics (`FallbackStats`) |

- **Le contrat backend** : `GraphicsBackend::capabilities()` — un futur backend natif DÉCLARE ce qu'il offre et explique ses choix ; le mock du banc d'essai rend un rapport déterministe (timestamps coupés avec raison — la preuve qu'aucune feature optionnelle n'est supposée), aux limites abaissables par test (les refus device verrouillés).
- **Plateformes** : Metal/macOS est la validation CONTINUE (runs réels à chaque sous-phase) ; **Windows (DX12/Vulkan via wgpu) est un checkpoint EXTERNE déclaré** — aucun code spécifique n'existe, le premier run y est auto-diagnostiquant (le rapport de capacités dit backend, limites et décisions) ; la checklist vit dans `docs/testing.md`. Linux et consoles : hors périmètre.
- **La suite stress & régression** (sous-phase 17) protège le tout : la SCÈNE CANONIQUE qui compose les douze domaines du renderer d'un coup aux comptes exacts, le churn de ressources, les tempêtes de resize, les pertes de surface scénarisées, le long run sans dérive mémoire, la performance bornée — noire-boîte, sur l'API publique (`docs/testing.md`, section 1bis ; le long run GPU : section 2quater, levier `CHAOS_DIAG_FRAME`).
- **Limites V1 EXPLICITES** : les défauts WebGPU seuls (pas d'élévation), pas de features optionnelles au-delà des timestamps, le choix d'adaptateur par défaut de wgpu (pas de préférence de puissance ni multi-GPU), pas de surfaces HDR étendues.

## Pipelines

Le pipeline est un concept du moteur, jamais un type wgpu — et depuis le Material System mature (sous-phase 6), **une RÉSOLUTION INTERNE** : plus aucun chemin de draw public ne consomme un `PipelineHandle` brut. Le consommateur décrit un MATERIAL (modèle + état + opacité) ; le renderer résout la permutation :

```
MaterialModel + état (double_sided, opacity) + format de la destination de passe
        │  cache de permutations (HashMap → PipelineHandle, dédupliqué)
        ▼
PipelineDescriptor généré (« chaos.material.{tag}[.double_sided][.transparent][.{format}] »)
        ▼
GraphicsBackend::create_pipeline — l'audience implémenteur de backend
```

- **Le cache déduplique** : deux materials au même modèle et au même état partagent le MÊME pipeline GPU (la démo le prouve : `demo.screen` réutilise la permutation de `demo.floor`). La permutation SURFACE est résolue à `create_material` (eager — un shader Custom invalide échoue à la création) ; les permutations de cibles se résolvent à la première passe qui les demande (lazy, comptées aux stats). `None ≠ Some(format_surface)` dans la clé (le renderer ne connaît pas le format surface) — dédup non parfaite, assumée.
- **WGSL est le langage shader officiel du moteur** (`ShaderSource::Wgsl`) — compilable vers SPIR-V via naga pour un futur backend maison ; l'enum accueillera d'autres formats.
- **Vertex layouts déclaratifs** : `PipelineDescriptor.vertex_layout: Option<VertexLayout>` (`None` = bufferless) + `instance_layout: Option<VertexLayout>` (le SECOND slot, à cadence Instance — l'instancing, sous-phase 12). Les layouts sont définis côté Chaos et convertis vers wgpu uniquement dans le backend ; `VertexLayout::packed(&[formats])` calcule locations/offsets/stride, `packed_at(base, cadence, …)` le généralise. UV/tangentes/skinning = des attributs de plus.
- **La cible couleur** : `color_target: Option<TextureFormat>` — `None` (défaut) = le format de la surface, `Some(format)` pour une cible hors écran (les API graphiques exigent le format exact).
- **La transparence** : `transparent: bool` (`with_transparency()`) — alpha blending + profondeur en LECTURE SEULE (le couplage V1 : une surface translucide ne doit pas occulter) ; sinon blend REPLACE, profondeur écrite. L'état est PILOTÉ par le contrat de `MaterialOpacity` (`blends()`) ; les permutations MASKED n'en diffèrent que par leur point d'entrée fragment (`with_fragment_entry("fs_masked")` — section « L'opacité et l'ordre de rendu »).
- **Le test de profondeur** : `depth_compare: DepthCompare` (`Less` défaut, `LessEqual` — `with_depth_compare()`) — le vocabulaire des fonds plein écran dessinés à la profondeur maximale exacte (le ciel). Le pipeline ciel est la première permutation NON-material du renderer : son propre cache par format de destination (section « L'environnement et le ciel »), hors du `PipelineKey` des materials.
- **Ressources material** : `.with_material()` ajoute le groupe(2) au layout — piloté par `MaterialModel::material_inputs()` (sections Bindings et Materials).
- **Culling** : la convention d'enroulement **CCW vu de l'extérieur** (`docs/architecture/math-conventions.md`) ; les materials pilotent l'état via `double_sided` (défaut : faces arrière cullées — le réglage 3D opaque standard).
- Côté backend (`backend/wgpu/pipeline.rs`) : création sous **error scope wgpu** — un WGSL invalide ou un pipeline incohérent devient une `ChaosError::Graphics` propre, jamais un panic. Stockage en `Vec`, handle = index (les pipelines sont permanents, le cache borne leur nombre au nombre de permutations réellement utilisées).
- Exécution : `encode_pass` rejoue les `FrameDraw` de chaque passe du plan ; un handle inconnu est ignoré avec un `warn!`, jamais de panic.
- Les draws soumis vivent dans la **RenderQueue de leur passe** (section dédiée ci-dessous) avec la **durée de vie d'une frame de simulation** : le moteur vide toutes les files au début de chaque update (`clear_draws`), et toutes les présentations intermédiaires (rafales de redraw du resize interactif) re-présentent les mêmes files — jamais de frame vide entre deux updates. Le scene graph alimentera ces files plus tard.

## Render Queue

Chaque passe déclarée possède SA **`RenderQueue`** (`queue.rs`) — le concept qui transforme une succession de draw calls improvisés en rendu organisé. `Renderer::queue_draw` alimente la file de la passe principale, `queue_draw_to(pass, …)` celle d'une passe déclarée :

- **Contrat** : la queue reçoit les soumissions en **ordre de scène** et rend l'**ordre de rendu** (`ordered()`) ; chaque passe du `FramePlan` arrive au backend **déjà triée** et le backend exécute aveuglément. La politique (l'ordre) appartient au moteur, la mécanique (l'exécution) au backend.
- **Clé actuelle : le (material, mesh)** — tri **stable** (`sort_by_key`) : le material implique pipeline et bind group, le mesh ses buffers — le regroupement minimise les changements d'état ET forme les runs que l'instancing automatique fusionne (section « L'instancing ») ; l'ordre de soumission est préservé à clé égale (déterminisme).
- **La partition à quatre temps** (sous-phases 6 et 11) : à la résolution, opaques → masked → ciel → TRANSPARENTS TRIÉS par profondeur (arrière → avant, la caméra de la passe — section « L'opacité et l'ordre de rendu ») ; le regroupement par material est préservé dans les classes qui écrivent la profondeur, sacrifié chez les transparents (la correction avant le batching). Les draws hors du frustum de la passe sont rejetés AVANT la partition (sous-phase 13 — section « La visibilité »).
- Le backend saute les `set_pipeline` **et** les `set_bind_group` material redondants entre draws consécutifs — le tri par material rend les deux économies effectives (N draws d'un même material = 1 bind de pipeline + 1 bind de material).
- **Extensions prévues** — la clé grandit, le contrat ne change pas : tri par profondeur des transparents, tri composite ; optimisations notées : buckets/dirty-flag, skip des binds de buffers (mesh partagé), instancing, dynamic offsets. Les passes de rendu ont atterri (sous-phase 5) : une file PAR passe.
- Pure structure CPU, testée sans GPU (stabilité, regroupement, cycle de vie).

## Profondeur

Le depth buffer est de la **pure mécanique de backend** : ni `FramePlan`, ni `Renderer`, ni le trait `GraphicsBackend`, ni les shaders n'en savent rien — l'occlusion 3D est devenue correcte sans toucher un seul type public.

- **Format** : `Depth32Float` (`backend/wgpu/depth.rs`), le format profondeur portable de référence.
- **Cycle de vie** : la texture suit la surface — créée à l'init, recréée au resize (la garde 0×0 suspend le rendu avant d'y arriver), droppée avec le backend. `Lost`/`Outdated` reconfigurent la surface sans changer les dimensions : la vue reste valide.
- **La passe** : les opérations sont DÉRIVÉES du plan (sous-phase 5) — clear à `1.0` par défaut (le plus lointain — profondeur wgpu 0..1, nos conventions) et `store: Discard` (optimal sur GPU tile-based) ; une passe ultérieure de la même destination en `Keep` force le `Store` de la précédente et charge (`Load`) au lieu d'effacer — jamais de profondeur indéfinie (section « L'orchestration des passes »).
- **Les pipelines** : comparaison `Less` (plus proche = plus petit) — `LessEqual` pour le ciel dessiné à la profondeur maximale exacte (sous-phase 9, `DepthCompare`) ; l'écriture suit l'opacité — les permutations TRANSPARENTES testent sans écrire (sous-phase 6). Les pipelines sans test de profondeur (UI, post-process) arriveront avec leur premier consommateur ; le **reverse-Z** est noté comme future optimisation de précision.

## Shaders

Cinq réponses, une organisation minimale mais durable :

| Question | Réponse |
|---|---|
| Où ils vivent | `chaos_renderer/shaders/*.wgsl` — de vrais fichiers, embarqués à la compilation (`include_str!`), zéro I/O runtime |
| Comment identifiés | `ShaderLibrary` : noms nommespacés (`chaos.` pour les intégrés), constantes `shaders::builtin::*` — jamais de littéraux éparpillés. Intégrés : `chaos.vertex_color` (position+couleur), `chaos.textured` (position+UV, échantillonne le material du groupe 2), `chaos.lit` (position+normale+UV, ambiante + Lambert + ombres), `chaos.pbr` (Cook-Torrance GGX sur les 7 slots material + IBL + ombres), `chaos.sky` (le fond plein écran qui échantillonne la cubemap d'environnement) et `chaos.shadow` (la passe d'ombre : profondeur seule, vertex uniquement, groupe(0) réduit) |
| Ce qu'ils attendent | la convention `shaders::inputs` (groupes/slots) — verrouillée par le test naga des conventions |
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

## Textures

Première ressource de la phase V3 : la texture est un **concept moteur**, jamais un type wgpu exposé. Même patron que les buffers — descripteur possédé, handle générationnel, mécanique confinée au backend :

- **`TextureDescriptor`** (`label`, `width`/`height`, `format`, `usage`, `kind`, `mips`, `pixels: Vec<u8>`) — les pixels sont uploadés à la création au **layout niveau-majeur** (pour chaque niveau de mip : toutes les couches, rangées serrées, origine en haut à gauche — convention verrouillée dans `math-conventions.md`). Constructeurs `sampled(...)`, `render_target(...)`, `cube(...)` ; builder `with_mips(...)` ; `expected_total_byte_len()` donne la taille exacte attendue (niveaux × couches).
- **`TextureFormat`** : `Rgba8UnormSrgb` (couleurs destinées à l'affichage — albedo, UI), `Rgba8Unorm` / `R8Unorm` / `Rg8Unorm` (données linéaires — normal maps, roughness/metallic), **`Rgba16Float`** (le HDR des environnements — filtrable partout). Le choix sRGB vs linéaire appartient au descripteur : le shader recevra toujours des valeurs linéaires. **Absences documentées V1** : `Rgba32Float` (non filtrable sans feature backend), formats compressés (BC/ASTC — avec l'asset pipeline).
- **`TextureKind`** : `D2` (défaut) ou `Cube` (6 faces CARRÉES, ordre +X, -X, +Y, -Y, +Z, -Z). **Limite V1 documentée** : les materials n'échantillonnent que la 2D — un material sur cubemap est REFUSÉ avec erreur claire ; la passe environnement consommera les cubemaps.
- **`TextureMips`** : `None` (défaut), `Provided(n)` (niveaux fournis, validés à l'octet près), `Generate` (chaîne complète box-filter CPU — formats RGBA 8 bits et `Rgba16Float` (moyenne en f32 : le HDR au-delà de 1 ne s'écrête pas), textures 2D et CUBEMAPS (chaque face filtrée indépendamment — coutures possibles aux derniers niveaux, limite V1) ; résolue en `Provided` AVANT le backend : le backend ne voit jamais `Generate`).
- **`TextureUsage`** : `Sampled` ou `RenderTarget` — une cible de rendu refuse cube et mips en V1 (les chaînes de RT viendront avec leurs passes). C'est l'usage de la texture COULEUR des cibles hors écran (section « Render targets »).
- **Mise à jour CONTRÔLÉE** (`Renderer::update_texture`) : remplace les pixels du niveau 0 — texture 2D mono-niveau seulement (recréer pour le reste), jamais un fallback builtin, octets exacts, validé AVANT le backend.
- **Fallbacks par usage** (`BuiltinTexture`) : `chaos.white` (albédo — le fallback des materials), `chaos.black` (masques/émissifs), `chaos.normal_flat` (la normale plate du futur PBR) — lazy, partagés, PROTÉGÉS (ni destruction ni mise à jour).
- **Le descripteur porte ses règles de cohérence** (`validate()`) : dimensions non nulles, cube carré, chaîne de mips bornée et comptée à l'octet, generate limité à son domaine — des erreurs explicites avec l'attendu et le reçu. Le Renderer l'applique avant tout appel GPU ; le futur asset pipeline pourra valider sans Renderer (`mip_dimensions`/`max_mip_levels` publics).
- **`TextureHandle` générationnel** — mêmes garanties que les buffers ; le registre de durée de vie retient les MÉTADONNÉES (dimensions, format, kind, niveaux).
- Côté backend (`backend/wgpu/texture.rs`) : `device.create_texture` + un `write_texture` PAR NIVEAU (toutes couches d'un coup — le layout niveau-majeur s'y prête) sous error scope, pool générationnel dédié.
- **Versant CPU** : `rgba8_bytes_of` / `srgb8_bytes_of` (couleurs) et **`rgba16f_bytes_of`** (HDR — conversion binary16 maison, troncature documentée, zéro dépendance).
- **Destruction propre, deux chemins** (parité buffers) : `destroy_texture` explicite (retrait du pool + drop), ou drop du backend au shutdown. wgpu gère la libération différée côté GPU — détruire une texture encore référencée par une frame en vol est sûr. Aucune arithmétique du backend ne peut paniquer : les tailles de rangée saturent (`texel_row_bytes`) et wgpu rejette la copie sous error scope.
- **Versant CPU de l'upload** : `rgba8_bytes_of` / `srgb8_bytes_of` (patron `bytes_of_*` des buffers) convertissent des `Color` linéaires en texels — bruts pour les formats de **données**, encodés via la fonction de transfert sRGB de référence pour les formats d'**affichage** (l'alpha reste linéaire). C'est la règle anti « bug sRGB » : on n'écrit jamais du linéaire brut dans une texture sRGB. L'asset pipeline apportera ses propres octets décodés ; ces helpers servent le contenu procédural (démos, textures builtin, debug). Pas de mise à jour dynamique post-création (même règle que les buffers : elle viendra avec ses besoins réels).
- **Cache par clé logique** (`get_or_create_texture`) : la clé est le `label` — l'identité logique de la texture, où l'asset pipeline mettra le chemin d'asset. Hit → handle existant ; miss → création. Contrat V1 : **la clé fait foi, pas le contenu**. `destroy_texture` évince l'entrée correspondante (un get ultérieur recrée) ; `create_texture` reste le chemin brut qui crée toujours. Futurs préparés, pas codés : hot reload = remplacement sous la même clé, refcount/éviction = gestion mémoire GPU, streaming.
- La vue de texture est créée par le backend au moment du **binding** (son seul consommateur) ; le bind group la retient côté GPU.

## Samplers

Le sampler sépare la texture de la **manière dont elle est lue** — ressource moteur indépendante, un même sampler sert autant de textures que voulu :

- **`SamplerDescriptor`** (`label`, `filter`, `mip_filter`, `address_mode`, `anisotropy`) — `new()` donne les défauts standard (**Linear + Repeat**, mips `Nearest`, anisotropie 1). `with_filter(Nearest)` pour le pixel-art, `with_mip_filter(Linear)` pour le trilinéaire, `with_anisotropy(n)` (1..=16 — au-delà de 1, tout doit être Linear : la règle des API graphiques, VALIDÉE avant le backend), `with_address_mode` (`Repeat`, `ClampToEdge`, `MirrorRepeat`).
- **`SamplerHandle` générationnel**, mêmes garanties que buffers/textures (handle périmé → erreur explicite).
- Côté backend (`backend/wgpu/sampler.rs`) : `device.create_sampler` sous error scope (mipmap filter + anisotropy clamp transmis), pool dédié. Les **samplers de comparaison** (ombres) étendront le descripteur avec leur besoin réel.

## Bindings

Le système de binding est la **convention à trois étages** — les shaders déclarent, le moteur fournit. L'autorité exécutable est **`shaders::inputs`** (constantes de groupes et de slots) : le backend les consomme (aucun littéral de groupe/slot dans le code) et le test naga `builtin_shaders_follow_the_input_conventions` échoue en CI si un shader intégré déclare un binding hors convention.

| Groupe WGSL | Contenu | Géré par |
|---|---|---|
| `@group(0)` | `FrameUniforms { view_projection, camera_position, inverse_view_projection, environment_params }` (0) + `LightsUniforms` (1) + cubemap d'environnement (2) + son sampler (3) + shadow map (4, `texture_depth_2d`) + son sampler de COMPARAISON (5) | moteur, automatique (1×/frame) |
| `@group(1)` | `ObjectUniforms { model, normal }` | moteur, automatique (slot par draw) |
| `@group(2)` | ressources **material**, 7 SLOTS FIXES toujours remplis : base color (0), sampler (1), `MaterialUniforms { base_color, params, emissive }` 48 o (2), metallic/roughness (3), normal map (4), occlusion (5), émissif (6) — fallbacks neutres par slot | contenu, via Material |

- Le groupe(2) appartient au **Material** (section suivante) — un seul layout de groupe(2), une seule voie de dessin. Côté backend, `MaterialBindingDescriptor` (texture/sampler résolus + base_color) → vue + buffer 16 o + bind group sous error scope, pool générationnel ; le bind group retient vue et sampler côté GPU — détruire la texture source ensuite est sûr.
- **Le buffer d'uniforms est RETENU à côté du bind group** (sous-phase 6) : `update_material_binding` (trait backend) écrit les paramètres EN PLACE — `set_material_color` ne recrée rien. Changer une texture/un sampler recrée le seul binding (les bind groups wgpu sont immuables) ; jamais le pipeline.
- **Opt-in par pipeline** : le groupe(2) suit `MaterialModel::material_inputs()`. Dérives observables, jamais fatales : binding manquant ou périmé → draw écarté avec `warn!` ; un pipeline sans groupe material ignore simplement le binding du material (cas normal : `chaos.vertex_color` n'en lit pas).
- **Extensibilité** : les paramètres material grandiront dans `MaterialUniforms` (metallic/roughness pour le PBR) sans changer la convention ; le skip des binds redondants est l'optimisation notée.

## Materials (mature V1 — consolidation, sous-phase 6)

Le material est **LA couche visuelle** du moteur : il DÉCRIT une surface — modèle, paramètres, textures, état de rendu, opacité — sans jamais être réduit à un pipeline ni à des bindings backend. Le draw reste le triplet classique :

```
DrawCommand { mesh, material, transform }
   mesh      = quelle géométrie          (MeshHandle — son vertex layout est VALIDÉ contre le modèle)
   material  = quelle apparence          (MaterialHandle : modèle + paramètres + textures + état)
   transform = où dans le monde          (Transform)
```

**Le modèle (`MaterialModel`)** — la famille de shaders et son contrat :

| Modèle | Shader | Layout attendu | Entrées material (groupe 2) |
|---|---|---|---|
| `VertexColor` | `chaos.vertex_color` | `ColorVertex` | non — texture/sampler/base_color REFUSÉS (jamais inertes en silence) |
| `Unlit` | `chaos.textured` | `TexturedVertex` | oui — `sample × base_color` |
| `Lit` | `chaos.lit` | `LitVertex` | oui — `sample × base_color × (ambiante + Lambert)` (section « L'éclairage ») |
| `Pbr` | `chaos.pbr` | `LitVertex` | oui + propriétés PBR (section « Le matériau PBR ») |
| `Custom { shader, vertex_layout, material_inputs }` | celui de l'app | déclaré | déclaré |

- **`MaterialDescriptor`** (`new(label, model)` + `with_base_color` / `with_texture` / `with_sampler` / `double_sided()` / `with_opacity()`) → `create_material` → `MaterialHandle` générationnel. Texture et sampler **optionnels** : fallbacks builtin `chaos.white` et `chaos.default_sampler`, lazy et partagés. AUCUN pipeline : la permutation se résout en interne (section Pipelines) — la permutation surface immédiatement (un shader Custom invalide échoue à la création), celles des cibles à la première passe qui les demande. Un MÊME material sert la surface ET les cibles hors écran (la démo : `demo.solid` dessine la ronde dans les deux passes).
- **L'état de rendu** : `double_sided` (défaut : faces arrière cullées) ; **l'opacité** : `Opaque`, `Masked` (alpha cutout — `alpha_cutoff`, section « L'opacité et l'ordre de rendu ») ou `Transparent` (alpha blending, profondeur en lecture seule, rendu APRÈS les opaques de sa passe, TRIÉ arrière → avant). Modèle et état sont FIGÉS à la création — recréer le material pour en changer (le cutoff, lui, se règle à chaud : `set_material_alpha_cutoff`).
- **La mise à jour contrôlée** : `set_material_color` écrit le buffer d'uniforms EN PLACE (16 octets, zéro recréation — le chemin par frame : la démo fait pulser l'alpha de son verre) ; `set_material_texture`/`set_material_sampler` recréent le SEUL binding, transactionnellement (validations d'abord, l'ancien binding part en retraite, les parts de refcount déplacées) — le handle SURVIT : même identité, nouvelle apparence. Même texture/sampler = no-op.
- **L'inspection** : `material_info(handle)` → `MaterialInfo` (label, modèle, paramètres courants, ressources résolues, état) — la matière du futur éditeur, reflète les mises à jour.
- **Les contrats sont validés, jamais silencieux** : entrées material sur un modèle qui ne les consomme pas → refus nommé ; cubemap → refus (V1) ; au resolve, un mesh au layout désassorti du modèle → draw écarté avec warn, un material qui échantillonne la destination de SA passe (feedback, même introduit par `set_material_texture`) → écarté.
- **Possession** : le material possède son binding GPU (groupe 2 — créé pour TOUS les materials, ignoré par les pipelines sans entrées) ; texture et sampler référencés sont partagés et comptés. La **RenderQueue** trie par material ; deux materials au même modèle/état partagent le MÊME pipeline (cache).
- **L'évolution vers le material PBR a eu lieu par addition**, comme cartographiée dans `docs/renderer/lighting-preparation.md` : les modèles `Lit` (sous-phase 7) et `Pbr` (sous-phase 8), les 7 slots fixes, l'IBL d'environnement (sous-phase 9) — zéro rupture du descripteur ni du triplet de draw.

## Uniforms

Le moteur parle en matrices et Transforms — jamais en bind groups. Convention de binding du moteur (généralisable : matériaux → group 2) :

| Groupe WGSL | Contenu | Fréquence | Mécanique backend |
|---|---|---|---|
| `@group(0)` binding 0 | `FrameUniforms { view_projection, camera_position, inverse_view_projection, environment_params }` | 1× par passe | buffer 160 o unique, `queue.write_buffer` (l'inverse calculée au packing — la déprojection du ciel) |
| `@group(0)` binding 1 | `LightsUniforms` (ambiante + compte + 16 lumières + la queue OMBRE : matrice de lumière @1056, paramètres @1120) | 1× par plan | buffer 1 136 o unique, MÊME bind group |
| `@group(0)` bindings 2–3 | la cubemap d'ENVIRONNEMENT + son sampler interne | rebindé à `set_environment` | vue Cube + bind group frame RECONSTRUIT (cube fallback noir sinon) |
| `@group(0)` bindings 4–5 | la SHADOW MAP + son sampler de COMPARAISON interne | rebindé à `set_shadow` | vue depth + bind group frame RECONSTRUIT (fallback 1×1 à 1.0 sinon — les vues vives sont RETENUES : rebinder l'ombre ne perd jamais l'environnement) |
| `@group(1)` | `ObjectUniforms { model, normal }` | 1× par draw | pool de slots 128 o (buffer + bind group), réutilisés par index de draw, agrandi à la demande |
| `@group(2)` | ressources material (sections Bindings/Materials) | par draw, si le pipeline l'a demandé | bind groups des materials, pool générationnel |

- Côté abstraction : `Renderer::set_view_projection(Mat4)` (la caméra le fournit — chaque passe déclarée porte la sienne) et `DrawCommand.transform: Transform` (résolu en matrice modèle au plan). Les uniforms restent de la mécanique interne pilotée par le plan — la seule méthode backend gagnée est `set_environment` (le rebind de la cubemap, sous-phase 9).
- Tous les pipelines reçoivent le layout standard `[frame, objet]` ; `mat4_to_bytes` convertit sans allocation (column-major glam = layout WGSL).
- **Le buffer frame et les slots objets sont PARTAGÉS entre les passes** : chaque passe écrit ses uniforms puis SOUMET (un submit par passe) — la timeline de queue garantit qu'une écriture stagée après un submit s'applique après ses commandes. C'est le contrat de correction du multi-passes (sous-phase 5).
- Optimisation prévue pour le render queue : dynamic offsets sur un buffer unique au lieu d'un slot par draw (et d'un submit par passe).

## Géométrie

La géométrie est une **donnée moteur**, distincte de sa représentation GPU et de son usage :

| Couche | Type |
|---|---|
| Données CPU | `Geometry` (`geometry.rs`) : `Vec<ColorVertex>` + indices u16 (vide = non indexé) ; constructeurs `triangle(center, size, colors)` / `quad(center, w, h, color)` / `cube(center, size, face_colors)`. **`TexturedGeometry`** : `Vec<TexturedVertex>` (position + UV, origine haut-gauche) ; `quad`/`cube`. **`LitGeometry`** : `Vec<LitVertex>` (position + normale + UV) ; `quad`/`cube` (les normales de face conservées) et **`sphere(center, radius, segments, rings)`** (UV-sphère, normales radiales, résolution clampée sous la limite u16 — jamais d'écrasement silencieux). L'unification des trois viendra avec l'asset pipeline |
| Représentation GPU | buffers créés depuis `vertex_bytes()`/`index_bytes()` (étape 6) |
| Usage | `DrawCommand { pipeline, vertex_buffer, index_buffer, element_count }` — `index_buffer` présent → `draw_indexed` (Uint16), sinon `draw` |

Le cube est la première géométrie **fermée** : 24 sommets (4 par face — une couleur par face exige des sommets non partagés, la topologie qu'exigeront normales/UVs), 36 indices, faces ordonnées **+X, -X, +Y, -Y, +Z, -Z**, enroulement **CCW vu de l'extérieur** (convention verrouillée par test — voir `docs/architecture/math-conventions.md`). Depuis l'étape 8, toute géométrie de la démo est construite **à l'origine** et placée exclusivement par le `Transform` de son `DrawCommand` ; le paramètre `center` des constructeurs reste disponible pour cuire un offset local quand c'est pertinent.

## Meshes

Le mesh est la **ressource de rendu de première classe** du moteur — c'est elle que consommeront asset system, scènes, ECS, éditeur et l'API de contenu (primitives aujourd'hui ; glTF, assets importés, outils et contenu utilisateur demain : tous aboutiront à `create_mesh`).

```
Geometry / TexturedGeometry ──► create_mesh / create_textured_mesh ──► MeshHandle
                                          │ le mesh POSSÈDE ses buffers GPU
DrawCommand { mesh, material, transform } ┤ Renderer::queue_draw
                                          ▼ résolution au render_frame (registres générationnels)
        FrameDraw { pipeline, buffers, element_count, model, binding } ──► backend
```

- **Le mesh vit dans l'abstraction** : le backend ne connaît toujours que buffers et pipelines. Le `Renderer` tient le registre (pool générationnel partagé, `src/pool.rs`) et résout mesh → buffers en construisant le plan.
- **Un mesh = une ressource, un draw = un usage** : le même `MeshHandle` peut être soumis N fois par frame avec des transforms différents — mêmes buffers GPU, une matrice modèle par draw (slot d'uniform par index, réutilisé chaque frame). Verrouillé par test mock ; la ronde de la démo dessine 8 cubes d'un seul mesh. L'instancing GPU sera l'optimisation de ce motif.
- **Durée de vie** : `destroy_mesh` détruit le record ET ses buffers ; handle périmé → erreur explicite ; un draw sur mesh détruit est écarté du plan avec un `warn!`, jamais de panic.
- Le record porte le **vertex format** (`VertexLayout`) et, depuis la sous-phase 13, les **bounds locaux** (`Option<Aabb>`, calculés à la création — `mesh_bounds()` pour l'inspection, section « La visibilité »). La validation croisée pipeline↔mesh est notée pour plus tard.
- `create_buffer`/`destroy_buffer` restent publics pour les usages avancés, mais les apps parlent mesh.

Présentation : `AutoVsync` si `RendererConfig::vsync` est actif, `AutoNoVsync` sinon — le défaut du moteur est vsync **off**, car un present bloquant sur le main thread rend les interactions fenêtre laggy sur macOS (winit #1737) ; la cadence est régulée par `target_fps` côté moteur. Delta d'horloge borné côté moteur.

## Couleur

`chaos_core::Color` (RGBA f32 linéaire) est le type du vocabulaire moteur ; la conversion vers le type du backend se fait à la frontière (`to_wgpu_color`). `EngineConfig::clear_color` contrôle la couleur de fond.

## Ce que les phases futures brancheront ici

- **Ombres** : Shadows V1 a ATTERRI (section « Les ombres ») — restent les extensions notées : cascades/fitting caméra, ombres ponctuelles/spots (`ShadowConfig` est le point d'extension), bias rasterizer, casters alpha-masqués ; côté IBL, le préfiltre GGX et la BRDF LUT raffineront les approximations V1.
- **Post-process** : les briques existent (cibles hors écran + passes déclarées + `Keep`) — la passe post-process viendra avec son besoin réel. Un **render graph avancé** (tri topologique, ressources transitoires) pourra remplacer l'ordonnancement explicite plus tard SANS changer le contrat backend : le plan restera la suite ordonnée de passes.
- **Asset pipeline** : images décodées → `TextureDescriptor` (clé de cache = chemin d'asset), meshes importés → `create_mesh`/unification des géométries.
- **Mode headless** (serveur dédié) : un backend nul derrière le même trait.
