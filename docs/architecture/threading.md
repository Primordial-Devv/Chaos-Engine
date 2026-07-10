# La politique de threads

La règle d'or : **aucun thread n'est lancé aujourd'hui** — aucun cas d'usage réel ne l'exige. Cette politique ne construit pas le multithreading : elle garantit qu'AUCUNE décision actuelle ne le rend impossible. Chaque règle est verrouillée au niveau le plus fort disponible : la compilation quand c'est possible, la CI sinon, la politique documentée en dernier recours.

## La carte des règles

| Domaine | Règle | Verrou |
|---|---|---|
| Boucle OS, fenêtre | **main thread À VIE** — macOS l'impose (`run_app` ne rend jamais la main) ; les opérations de fenêtre restent sur le main thread par politique, même quand winit les tolère ailleurs | politique + confinement winit dans `chaos_window` |
| `Engine::run` fenêtré | main thread (piloté par la boucle OS), une fois par processus | politique (contrainte OS) |
| `Engine::run` headless | **LIBRE** — aucune boucle OS, aucune contrainte de thread | documenté (mode headless) |
| Subsystems, `EngineContext` | **main thread, `&mut` séquentiel** — l'unité d'ORCHESTRATION, jamais de parallélisme ; le parallélisme viendra de l'INTÉRIEUR (systèmes, futurs jobs), pas du déplacement des subsystems | politique documentée — l'absence de borne `Send` est DÉLIBÉRÉE (état local libre) |
| Systèmes ECS | `System: Send + Sync` PAR CONTRAT — l'unité FUTURE du parallélisme ; la frontière de stage est le futur point de synchronisation (déjà documenté dans `Schedule`) | compilation (supertrait) |
| `World`, `Component`, `Resource`, `Message` | `Send + Sync` par contrat | compilation (supertraits + verrou `the_world_is_send_and_sync`) |
| `Schedule` | `Send + Sync` (conséquence du contrat des systèmes) | compilation (verrou `the_schedule_is_send_and_sync`) |
| `SceneManager` | `Send + Sync` | compilation (verrou existant) |
| `Renderer`, `GraphicsBackend` | **`Send`** — la porte du futur render thread : le plan de frame se construit côté simulation, la `RenderQueue` est déjà le point de découplage (`clear_draws`/`ordered`) | compilation (`GraphicsBackend: Send` + verrou `the_renderer_is_send`) |
| `AssetManager`, `AssetImporter` | **`Send + Sync`** — la porte du futur chargement asynchrone/streaming | compilation (`AssetImporter: Send + Sync` + verrou `the_manager_is_send_and_sync`) |
| État global | **INTERDIT** (`static mut`, once_cell, thread_local) — le prérequis n°1 du multithreading sain | CI (`chaos_engine/tests/boundaries.rs`) |

## Ce que le compilateur refuse dès aujourd'hui

Un `Rc`, un `RefCell` partagé ou tout type non-`Send` dans : un système ECS, un composant, une ressource, un message, un importeur d'assets, un backend graphique. Ces types sont les futurs voyageurs entre threads — le contrat est posé pendant que l'API est jeune, le compilateur est le gardien.

## Les deux futurs préparés (jamais implémentés ici)

- **Le render thread** : `Renderer: Send` permet de déplacer le rendu ; la couture existe déjà — la simulation soumet dans la `RenderQueue`, la présentation consomme le plan de frame. Il manquera un double-buffer de plans et un canal, rien d'architectural.
- **Les systèmes ECS parallèles** : `System: Send + Sync` + la frontière de stage comme point de synchronisation. Il manquera le graphe d'accès (quelles données chaque système lit/écrit) — le contrat `Send + Sync` garantit qu'aucun système existant ne devra être réécrit.

## Ce qui ne changera pas

Le main thread reste le chef d'orchestre : boucle OS, fenêtre, cycle de vie des subsystems, hooks séquentiels. La question du multithreading n'est jamais « comment paralléliser l'orchestration » mais « comment paralléliser le TRAVAIL » (systèmes, jobs, rendu) — l'orchestration séquentielle est ce qui rend le reste sûr.
