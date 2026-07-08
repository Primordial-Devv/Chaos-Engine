# Renderer — architecture

Référence des choix de la phase 2 (renderer minimal). Le principe directeur : **wgpu est un détail d'implémentation, jamais une dépendance du moteur.**

## Les deux étages

```
chaos_engine ──► Renderer (API orientée moteur, vocabulaire chaos_core)
                   └─► trait GraphicsBackend (le point de remplacement)
                         └─► WgpuBackend (seul module qui importe wgpu)
```

- **`Renderer`** (`renderer.rs`) — ce que voit le moteur : `attach(target, w, h)`, `resize`, `set_clear_color`, `render_frame`, `description`. Ne parle que le vocabulaire de `chaos_core` (`Color`, `ChaosResult`).
- **`GraphicsBackend`** (`backend.rs`) — le contrat qu'un backend doit honorer. Remplacer wgpu par un backend maison (Vulkan, DirectX 12, Metal) = implémenter ce trait, rien d'autre à toucher dans le moteur.
- **`WgpuBackend`** (`backend/wgpu_backend.rs`) — l'unique fichier du workspace qui importe wgpu. Détient surface, device, queue et configuration.

## La couture avec la fenêtre : raw-window-handle

`chaos_renderer` ne dépend pas de `chaos_window` (règle d'architecture : sous-systèmes → core uniquement). Le pont est le standard d'interop `raw-window-handle` :

- `chaos_window::WindowHandle` implémente `HasWindowHandle`/`HasDisplayHandle` (délégation à winit) ;
- `chaos_renderer::SurfaceTarget` accepte toute cible exposant ces handles (impl blanket) ;
- seul `chaos_engine`, qui voit les deux crates, passe la fenêtre au renderer (`RenderSubsystem`).

## Intégration au cycle de vie du moteur

Le renderer est le **premier vrai Subsystem**. `RenderSubsystem` (dans `chaos_engine`) l'adapte au trait :

| Hook | Action |
|---|---|
| `init` | `Renderer::attach` (échec → chemin d'erreur d'init standard → arrêt propre) |
| `on_event` `Resized` | `renderer.resize(w, h)` |
| `render` | `renderer.render_frame()` (erreur fatale → `request_exit`) |
| `shutdown` | libération des ressources GPU |

Il est enregistré automatiquement par l'Engine à l'ouverture de la fenêtre, en dernier : il présente après les updates de tous les subsystems, et le shutdown en ordre inverse le détruit en premier.

Le rendu est piloté par `RedrawRequested` (hook `on_redraw` → phase render), pas par la boucle d'update — indispensable pour rester fluide pendant le resize interactif macOS. `on_update` se termine par `request_redraw()`.

## Cycle d'une frame (backend wgpu)

1. `get_current_texture()` → `CurrentSurfaceTexture` : `Success`/`Suboptimal` → on présente ; `Lost`/`Outdated` → reconfiguration + frame sautée ; `Timeout`/`Occluded` → frame sautée ; `Validation` → erreur fatale.
2. Render pass unique avec `LoadOp::Clear(clear_color)`.
3. `queue.submit` puis `queue.present`.

Robustesse : resize 0×0 ignoré (minimisation Windows), delta d'horloge borné côté moteur. Présentation : `AutoVsync` si `RendererConfig::vsync` est actif, `AutoNoVsync` sinon — le défaut du moteur est vsync **off**, car un present bloquant sur le main thread rend les interactions fenêtre laggy sur macOS (winit #1737) ; la cadence est régulée par `target_fps` côté moteur.

## Couleur

`chaos_core::Color` (RGBA f32 linéaire) est le type du vocabulaire moteur ; la conversion vers le type du backend se fait à la frontière (`to_wgpu_color`). `EngineConfig::clear_color` contrôle la couleur de fond.

## Ce que les phases futures brancheront ici

- **Triangle / pipelines** : pipeline de rendu, shaders (WGSL), vertex buffers — extension du contrat `GraphicsBackend`.
- **Meshes, textures, matériaux, caméra, lumières** : nouvelles ressources exposées par `Renderer`, implémentées derrière le trait.
- **Post-process, render graph** : orchestration au niveau `Renderer`, opaque pour le moteur.
- **Mode headless** (serveur dédié) : un backend nul derrière le même trait.
