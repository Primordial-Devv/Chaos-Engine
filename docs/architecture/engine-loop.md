# Boucle moteur et cycle de vie

**Engine Core — mature V1** : la colonne vertébrale de Chaos Engine — déterministe (ordre de frame et de subsystems verrouillés par tests), observable (profiling CPU + metrics santé), configurable (validée avant toute init), résiliente (modèle d'erreurs, garanties d'arrêt), compatible headless (le même core, fenêtré ou non), prête pour les charges futures (politique de threads verrouillée à la compilation) — la fondation de Chaos Editor, du runtime et du serveur dédié. Référence des choix d'architecture : fenêtre, événements, boucle, cycle de vie, et toutes les consolidations. Tout ce qui suit conditionne les phases futures.

## Inversion de contrôle : qui possède la boucle ?

macOS impose que la boucle d'événements OS tourne sur le main thread à travers son API (`run_app` de winit ne rend jamais la main). Le moteur est donc construit en **inversion de contrôle** :

- **winit possède la boucle OS** — démarrée par `chaos_window::run_event_loop`.
- **`chaos_engine` possède la boucle logique** — le cycle update des subsystems, tické à chaque passage de la boucle OS.
- **`chaos_window` est la couture** : le trait `WindowEventHandler` (implémenté par `Engine`) transporte la vie de l'une vers l'autre.

L'alternative `pump_events` (boucle possédée par le moteur) a été rejetée : non portable, déconseillée par winit, fragile sur macOS.

## Confinement de winit

Aucun type winit ne sort de `chaos_window`. Les événements sont traduits à la frontière (`translate.rs`) vers le vocabulaire maison défini dans `chaos_core` (`Event`, `WindowEvent`, `InputEvent`, `KeyCode`, `MouseButton`, `ElementState`). Conséquences :

- remplacer winit ou ajouter un backend ne touche qu'une crate ;
- les futurs consommateurs (ECS, runtime, éditeur) dépendent de `chaos_core`, jamais de `chaos_window` — conforme à la règle « sous-systèmes → core uniquement ».

## La configuration

`EngineConfig` est LE modèle de configuration du moteur — celui que l'éditeur (Project Settings), le runtime, le serveur dédié et les builds consommeront. Chaque domaine est un groupe distinct :

| Groupe | Champs | Domaine |
|---|---|---|
| `app` | `name` | l'application lancée — les logs de démarrage/arrêt aujourd'hui, le nommage des builds demain |
| `window` | `title`, `width`, `height`, `resizable` | la fenêtre native (`WindowConfig`, de `chaos_window`) |
| `render` | `vsync`, `clear_color` | la politique de présentation — appliquée au renderer à son attachement |
| `time` | `target_fps`, `fixed_timestep` | la cadence de la boucle et le pas de la simulation fixe |
| `logs` | `filter` | la politique de logs — PORTÉE par la configuration, APPLIQUÉE par l'application (l'init du logger appartient au binaire, jamais au moteur) ; `RUST_LOG` garde la priorité |
| `runtime` | `headless`, `frame_limit`, `pause_on_focus_loss`, `disabled_subsystems` | le mode d'exécution global : headless (le mode réel, voir sa section), arrêt après N frames, pause auto à la perte de focus (voir « Pause, suspension et focus »), désactivation par nom |

Le cycle : **des défauts sûrs** (`Default`, groupe par groupe) → **la surcharge par l'application** (littéral + `..Default::default()` — le patron du sandbox) → **la validation par le moteur** (`EngineConfig::validate()`) → **la consultation contrôlée** (le moteur LIT la configuration, jamais ne la réécrit).

**Une configuration invalide échoue AVANT toute initialisation partielle** : `Engine::run()` valide EN PREMIER — avant la boucle d'événements, la fenêtre, le schedule et l'init des subsystems (`start()` valide au même point, quelle que soit la voie d'entrée). L'erreur est précise et exploitable (`ChaosError::Config`), jamais un rattrapage silencieux :

| Règle violée | Refus |
|---|---|
| `app.name` vide | nommé |
| dimension de fenêtre nulle | les dimensions fautives citées |
| `render.clear_color` non finie | les composantes citées |
| `time.target_fps == Some(0)` | « use `None` for an unpaced loop » |
| `time.fixed_timestep` nul | « must be strictly positive » |
| `logs.filter` vide | « use `None` to keep the logger default » |
| `runtime.frame_limit == Some(0)` | « use `None` to run unlimited » |
| désactivation dupliquée ou vide | le doublon nommé |

`runtime.disabled_subsystems` est CONSOMMÉ au démarrage : les subsystems listés sont retirés AVANT le tri des dépendances — jamais initialisés, jamais tickés (`info!` par retrait). Un nom qui ne correspond à aucun subsystem enregistré refuse le démarrage : une désactivation qui ne désactive rien est une erreur de configuration, pas un souhait ignoré.

## Cycle de vie du moteur

```
Engine::run()
  ├─ EngineConfig::validate()                AVANT TOUT : l'invalide échoue ici,
  │                                          sans boucle OS, sans fenêtre,
  │                                          sans init partielle
  ├─ (runtime.headless) → run_headless()     la boucle PROPRE du moteur :
  │                                          start() → on_update()×N →
  │                                          on_shutdown() — les MÊMES hooks,
  │                                          sans fenêtre ni GPU (voir la
  │                                          section « Le mode headless »)
  └─ run_event_loop()                        boucle OS démarrée
       ├─ on_window_ready(WindowHandle)      fenêtre native créée
       │    ├─ enregistrement du RenderSubsystem (en dernier)
       │    ├─ validation de la configuration (la même garde sur toute voie d'entrée)
       │    ├─ déclaration des stages ECS `stages::{UPDATE, LATE_UPDATE, POST_UPDATE}`
       │    │    └─ + le service moteur TransformPropagation (POST_UPDATE)
       │    ├─ retrait des subsystems désactivés (runtime.disabled_subsystems)
       │    ├─ retrait des subsystems graphiques si headless (requires_graphics)
       │    ├─ tri topologique par dépendances (Kahn stable)
       │    └─ init des subsystems           dans l'ordre TRIÉ
       │         └─ (ils y enregistrent leurs systèmes ECS)
       ├─ on_event(Event)                    système + entrées, traduits
       │    ├─ CloseRequested → request_exit
       │    └─ pompé en message ECS          world.send_message(event)
       ├─ on_update()                        chaque frame (about_to_wait) :
       │    ├─ gating : rien avant l'échéance de frame (target_fps)
       │    ├─ FrameClock::tick()            delta borné (max 250 ms)
       │    ├─ tick ECS : ressource Time rafraîchie + Schedule sur le World
       │    │             (UPDATE → LATE_UPDATE → POST_UPDATE — voir le
       │    │              modèle d'exécution ci-dessous)
       │    │             (échec → FRAME ABANDONNÉE : défaillance fatale
       │    │              stockée, arrêt ordonné, Err par run())
       │    ├─ clear_draws()                 la RenderQueue repart vide
       │    ├─ update de chaque subsystem    phase simulation (lit l'état ECS
       │    │                                frais, soumet les draws)
       │    ├─ clear_messages()              un message vit UN update
       │    ├─ frame_limit éventuel
       │    └─ request_redraw()
       ├─ frame_deadline()                   → ControlFlow::WaitUntil(échéance)
       ├─ on_redraw()                        sur RedrawRequested :
       │    └─ render de chaque subsystem    phase présentation
       └─ on_shutdown()                      subsystems arrêtés en ordre INVERSE
                                             (le renderer part ici — avant la fenêtre),
                                             puis scènes déchargées (déterministe),
                                             requêtes annulées, World remis à zéro,
                                             assets fermés, fenêtre relâchée EN DERNIER
                                             (les garanties : section dédiée)
```

La séparation update/render suit le modèle winit : la simulation vit dans
`about_to_wait`, la présentation dans `RedrawRequested` — ce qui garde le rendu
fluide pendant le resize interactif macOS (boucle modale). Détails du renderer :
`docs/renderer/overview.md`.

## Le mode headless

`runtime.headless = true` est un VRAI mode d'exécution — jamais une fenêtre invisible : ni boucle OS, ni fenêtre, ni GPU. La boucle logique COMPLÈTE tourne : ECS, scènes, assets, temps (variable ET fixe), machine d'états, subsystems. Serveur dédié, tests d'intégration, outils, CI sans GPU — c'est ce chemin.

**Zéro duplication : le même Engine Core orchestre les deux modes.** `run_headless()` pilote LES MÊMES hooks que la boucle OS — `start()` → `on_update()`×N → `on_shutdown()` — seul le driver change. Les différences sont des ABSENCES, pas des variantes :

- **pas de phase présentation** : `on_redraw` n'existe pas, `Subsystem::render` n'est JAMAIS appelé ;
- **`renderer()` reste `None`** — l'Option EST la frontière headless, inchangée ;
- **pas d'événements** : `on_event` ne fire jamais (aucune entrée OS) ;
- **la pause est indisponible** : la reprise arrive par les événements et ce canal n'existe pas — une requête de pause serait un gel définitif (même `frame_limit` gèle en pause) ; elle est écartée avec `warn!` ;
- **la cadence est tenue par sleep** (`time.target_fps`) : il n'y a aucun événement à pomper entre les ticks — `None` = boucle libre (le levier des tests).

**La classification des subsystems** — le moteur SAIT ce qu'est chacun :

| Catégorie | Mécanisme |
|---|---|
| **graphique** | `Subsystem::requires_graphics()` → retiré au démarrage en headless (`info!` par retrait), jamais initialisé ni tické |
| **compatible headless** | le défaut (`requires_graphics()` = `false`) |
| **optionnel** | `runtime.disabled_subsystems` (désactivation par configuration) |
| **obligatoire** | encodé par les DÉPENDANCES : un subsystem restant qui dépend d'un retiré refuse le démarrage (« depends on 'x' which is not registered ») — dépendre d'un graphique, c'est être graphique |

L'arrêt : `frame_limit` (un nombre défini de ticks — tests, CI, tâches d'import) ou `request_exit()` d'un subsystem (la sémantique serveur : non borné jusqu'à décision). Vérification vivante : `CHAOS_HEADLESS=1 CHAOS_FRAME_LIMIT=240 cargo run -p sandbox` (voir `docs/testing.md`).

## Les diagnostics et le profiling CPU

Le moteur peut expliquer OÙ passe son temps CPU pendant une frame — le service `context.diagnostics()`, en lecture seule :

| Mesure | Où |
|---|---|
| durée totale de la frame | `FrameProfile::total` — le MUR, tick à tick (`real_delta`, zéro lecture d'horloge en plus) |
| temps d'update | `update` — toute la simulation du tick (pas fixes + stages + subsystems + fin de frame) |
| temps de rendu CPU | `render` — la somme des hooks `render` (l'encodage/soumission, jamais le GPU) |
| temps par phase | `fixed` (bloc à pas fixe) + `stages` + `subsystems` + `renders` |
| temps par subsystem | `subsystems`/`renders` — un `Span` nommé par subsystem, dans l'ordre trié |
| temps par stage ECS | `stages` — un `Span` par stage variable, dans l'ordre d'exécution |
| nombre de frames | `frame_index` (+ la synthèse au shutdown) |
| dépassements de budget | `over_budget` par frame + `overruns()` cumulés — le TRAVAIL (`update + render`) contre le slot (`1/target_fps`), jamais le mur : l'attente de cadence n'est pas un dépassement |
| fréquence des fixed updates | `fixed_steps` par frame |

**Le modèle : un double-buffer de profils.** La frame COURANTE s'accumule pendant qu'elle s'exécute ; au tick suivant elle est close (sa durée murale = le `real_delta` mesuré) et échangée avec le snapshot. `last_frame()` rend TOUJOURS la dernière frame complète — jamais une frame à moitié remplie. Les frames en pause ne closent rien (le snapshot reste la dernière frame active) ; le `total` d'une frame traversée par une pause dit la vérité du mur.

**Le coût de la mesure** (elle ne perturbe pas ce qu'elle mesure) : ~10 paires d'`Instant::now()` par frame (~1 µs sur un budget de 16,6 ms), zéro allocation en régime établi (les slots nommés sont réécrits en place — les ensembles de stages et subsystems sont stables après le démarrage), zéro log sur le chemin chaud.

**Les consommateurs** : les logs de dev (la touche **O** du sandbox affiche le rapport `Display` à la demande ; une ligne de synthèse au shutdown : `diagnostics: N frame(s), M over budget`), les tests de performance (assertions sur le snapshot), le futur profiler de l'éditeur et les outils (le même service, lecture seule).

### Les metrics de santé

Le pendant SYNTHÉTIQUE et continu du profiling détaillé : `context.metrics().snapshot()` — l'état de santé consolidé, sans éditeur ni UI (le futur overlay debug, Chaos Editor, le serveur dédié et les diagnostics de production liront CE service) :

| Indicateur | Source |
|---|---|
| FPS + frame time moyen/min/max | une fenêtre glissante de 120 frames (ring buffer, zéro allocation — une écriture par frame), statistiques calculées à la LECTURE |
| nombre d'entités | `World::len()` |
| scènes actives | `SceneManager::actives()` |
| assets chargés | `AssetRegistry::loaded_count()` |
| draw calls | `Renderer::draw_count()` — 0 en headless |
| erreurs / avertissements | des compteurs CONTINUS incrémentés par les chemins moteur (`store_fatal`, refus explicites) — le lecteur DIFFE deux snapshots pour obtenir le « récent » |
| mémoire suivie | `AssetManager::cached_bytes()` (les octets bruts en cache) — la mémoire suivie LORSQUE disponible ; les tailles décodées viendront quand chaque type saura se mesurer |
| état des subsystems | décidé au démarrage : `Active` / `Disabled` (config) / `SkippedHeadless` |

**La cohérence** (le contrat du snapshot) : toutes les jauges sont échantillonnées à la FIN de la même frame, à l'état posé — le snapshot est daté (`frame_index`), jamais un mélange de frames. Chaque propriétaire compte le sien (trois accesseurs d'une ligne) ; le moteur échantillonne, il n'absorbe rien. Démo : la touche **H** du sandbox affiche le rapport santé.

## La frontière d'application

Les applications (sandbox, futur éditeur, futur serveur dédié, futur runtime) consomment UNE façade : `chaos_engine` — jamais une crate interne (**verrouillé en CI** : `tests/boundaries.rs`, manifestes ET imports ; les crates PLATEFORME restent permises — le modèle 4 couches `apps → platform → engine → foundation`). Le contrat, six points :

| Point | API |
|---|---|
| création | `Engine::new(config)` — UN cycle de vie par Engine (`run()` refuse un moteur déjà arrêté : « build a new one ») |
| configuration | `EngineConfig` groupé, défauts sûrs, surcharge par l'app, VALIDÉ avant toute init partielle |
| enregistrement | `add_subsystem` (ordre d'init) ; les extensions (importeurs, systèmes ECS) via les services du contexte pendant `Subsystem::init` |
| démarrage | `run()` bloquant — le MÊME core en fenêtré ou headless (`runtime.headless`) |
| demande d'arrêt | `request_exit` (subsystems), `frame_limit`, fermeture système — toujours l'arrêt ordonné garanti |
| résultats & diagnostics | le `ChaosResult` de `run()` (la première fatale, précise) + `Engine::diagnostics()`/`Engine::metrics()` lisibles APRÈS le run |

La ligne de partage : `EngineContext` est l'interface des SUBSYSTEMS (pendant les hooks), la façade `Engine` est celle des APPLICATIONS — le contexte entier ne s'expose pas. Le sandbox est LA démonstration vivante du contrat (fenêtré et headless, le même binaire), jamais un contournement — le verrou l'empêche de le devenir.

## Les garanties d'arrêt

L'arrêt n'est pas un chemin heureux : c'est une garantie architecturale. TOUS les scénarios convergent vers UN tunnel (`on_shutdown`, idempotent) — arrêt normal (`request_exit`), fermeture système (`CloseRequested`), défaillance fatale, échec partiel d'initialisation, arrêt headless, demande d'un subsystem, `frame_limit`. La séquence, ordonnée et verrouillée :

1. **subsystems en ordre INVERSE trié** — le `RenderSubsystem` relâche le renderer ICI, donc toujours AVANT la fenêtre ;
2. **scènes déchargées** (`SceneManager::shutdown`) — le déchargement DÉTERMINISTE : états, ordre, logs ; un échec est surfacé s'il est la seule défaillance ;
3. **requêtes en attente annulées** (pause pendante purgée) ;
4. **World remis à zéro** — la garantie PAR CONSTRUCTION : plus une entité (globales et persistantes comprises), plus une ressource, plus un message en attente ;
5. **assets fermés** (`AssetManager::shutdown`) — caches vidés, rétentions oubliées (l'arrêt prime sur la rétention), états `Loaded` → `Unloaded` ; les déclarations restent (métadonnées) ;
6. synthèse diagnostics, **fenêtre relâchée EN DERNIER**, état `Stopped`.

**L'invariant post-arrêt** (testé sur chaque scénario) : World vide, zéro message, scènes vides (`main() == None`), zéro asset chargé, zéro octet en cache, renderer absent — sans fuite logique, sans double destruction, sans subsystem encore actif.

- **Idempotence et silence** : un shutdown répété est un no-op ; après l'arrêt, `on_update`/`on_redraw`/`on_event` sont muets et `start()` est refusé — un moteur arrêté ne redémarre pas, on en construit un neuf.
- **Le slot de défaillance fatale SURVIT à l'arrêt** : `run()` le draine après le nettoyage — le diagnostic n'est jamais sacrifié à la propreté.
- **Le contrat de drainage** : aucun travail asynchrone n'existe encore ; quand les jobs arriveront, ils seront vidés ou annulés à l'étape 3 — le contrat est posé.

## Le modèle d'erreurs et de défaillances

Six catégories, chacune SON chemin — le comportement face aux échecs est cohérent, prévisible et testé :

| Catégorie | Chemin défini |
|---|---|
| **configuration** | échec AVANT toute initialisation partielle — `run()` → `Err` immédiat, ni boucle OS ni fenêtre |
| **initialisation** (init d'un subsystem, tri, schedule, attach renderer) | les inits suivants abandonnés, shutdown INVERSE des déjà-initialisés, `Err` précis par `run()` |
| **exécution FATALE** (échec du schedule ECS, échec de rendu, escalade `report_fatal`) | la frame est ABANDONNÉE si le monde est compromis (aucun code applicatif sur un état inconnu), arrêt ordonné à la frontière de frame, nettoyage complet, `Err` précis par `run()` |
| **propre à un subsystem, RÉCUPÉRABLE** | traitée LOCALEMENT — les services rendent `Result`, le subsystem gère (repli, log, réessai) ; le moteur CONTINUE, jamais d'arrêt sans raison |
| **échec au shutdown** (nettoyage des scènes) | best-effort : le nettoyage continue, l'échec est logué ; surfacé par `run()` seulement s'il est la SEULE défaillance |
| **demande d'arrêt NORMALE** (`request_exit`, `frame_limit`, fermeture fenêtre) | pas une erreur : arrêt ordonné, `Ok(())` |

Les règles transverses :

- **La première défaillance est LA cause** : elle seule ressort de `run()` ; les suivantes sont loguées comme conséquences (nommant la primaire) — jamais perdues en silence, jamais écrasantes.
- **L'escalade d'un subsystem** : `context.report_fatal(error)` — le pendant fatal de `request_exit` : diagnostic conservé, arrêt ordonné, remontée par `run()`. Les hooks d'exécution restent infaillibles (pas de `Result` sur `update`/`render`) : le récupérable se traite localement, le non-récupérable s'escalade explicitement.
- **Le diagnostic** : chaque erreur nomme son sujet (subsystem, système, stage, asset, chemin) ; les logs `error!` situent la défaillance (phase, frame) ; `run()` rend l'erreur brute exploitable.
- **Panic = bug HORS modèle** : `unwrap`/`expect` sont bannis du code moteur — le moteur ne panique JAMAIS sur un échec modélisé ; pas de `catch_unwind` (aucun besoin concret, coût de complexité réel).

## Le modèle d'exécution d'une frame

L'ordre est GARANTI et verrouillé par tests (`the_frame_follows_the_documented_execution_order`, `the_execution_order_is_identical_across_runs`) — identique entre les runs, sauf configuration explicitement différente. Les futurs systèmes savent précisément où s'exécuter :

| # | Responsabilité | Qui | Quoi |
|---|---|---|---|
| 0 | **Réception des événements** | moteur (`on_event`, entre les frames) | traduction OS → `chaos_core::Event`, pompe en messages ECS, dispatch aux subsystems — **visibles par la simulation de la MÊME frame** |
| 1 | **Préparation de frame** | moteur | requête pause/reprise appliquée (frontière déterministe), échelle de temps appliquée, tick d'horloge (delta borné puis échelonné), ressource `Time` rafraîchie |
| 1bis | **Simulation à pas fixe** | systèmes — `stages::FIXED_UPDATE` (schedule FIXE) | 0..N pas bornés par frame (rattrapage anti-spirale) ; la ressource `FixedTime` (pas constant) — jamais `Time` |
| 2 | **Simulation** | systèmes — `stages::UPDATE` | le jeu et le contenu : lire les messages, muter le monde (transforms LOCAUX compris) |
| 3 | **Mises à jour tardives** | systèmes — `stages::LATE_UPDATE` | réagir à la simulation (caméra qui suit, contraintes) — après TOUTES les mutations de jeu, AVANT la propagation : les écritures sont propagées la même frame |
| 4 | **Propagation** | systèmes-services — `stages::POST_UPDATE` | les dérivés : `GlobalTransform` par `TransformPropagation` |
| 5 | **Updates des subsystems** | subsystems (ordre d'enregistrement) | lire l'état POST-propagation, soumettre les draws (la RenderQueue a été vidée juste avant) |
| 6 | **Fin de frame** | moteur | `clear_messages` (un message vit UN update), `frame_limit`, pacing, `request_redraw` |
| 7 | **Rendu** | subsystems (`on_redraw`) | la présentation — découplée de la simulation (modèle winit) |

La préparation (1) et la fin de frame (6) sont des étapes MOTEUR, pas des points d'extension — un stage y viendra le jour où un besoin réel l'exigera.

## Le système de temps

Deux temps distincts, une frontière nette :

| Temps | Champs | Propriétés |
|---|---|---|
| **JEU** | `Time::{delta, elapsed}` | clampé (max 250 ms — jamais de pas géant après gel/breakpoint) puis ÉCHELONNÉ (`Time::scale`) — pilote la simulation |
| **RÉEL** | `Time::{real_delta, real_elapsed}` | brut, ni clampé ni échelonné — la vérité murale (profiling, timeouts) |
| **FIXE** | `FixedTime::{delta, elapsed, step_index}` | LE pas constant (`EngineConfig` → `time.fixed_timestep`, 1/60 par défaut) — les simulations déterministes |

- **L'échelle de temps** (`context.set_time_scale`) : appliquée à la frontière de frame ; non finie → refusée avec `warn!`, négative → 0. Elle échelonne le temps de JEU (le pas fixe compris — le slow-motion ralentit aussi la physique, voulu). **`scale = 0` n'est PAS la pause** : les systèmes tournent à delta nul, `frame_index` avance ; la pause, elle, gèle tout.
- **Le pas fixe** : l'accumulateur (`FixedClock`) transforme les deltas de jeu en 0..N pas par frame, **bornés** (5 max — l'anti-spirale de la mort) ; l'excédent au-delà du rattrapage est ABANDONNÉ : sous surcharge, la simulation ralentit au lieu de spiraler (le choix standard). Déterminisme : mêmes deltas → mêmes séquences de pas (testé). Les pas post-pause sont impossibles (horloge resynchronisée à la reprise — delta quasi nul —, aucune accumulation pendant la pause ; le clamp reste le filet des gels involontaires).

## Les services du contexte

`EngineContext` est une interface de SERVICES, pas un sac global. Chaque frontière est un contrat :

| Service | Accès | Frontière |
|---|---|---|
| Temps | `time()` (+ ressources `Time`/`FixedTime`) | instantané en lecture ; l'échelle par requête à frontière de frame |
| World ECS | `world()`/`world_mut()` | LES données vivantes ; **le canal de communication inter-subsystems** (messages/ressources) — jamais de dépendance directe entre subsystems |
| Scheduler | `schedule_mut()`/`fixed_schedule_mut()` | enregistrement à l'init (recommandé) ; post-init : appliqué à la frame SUIVANTE, déterministe à configuration identique |
| Assets | `assets()`/`assets_mut()` | l'unique point d'entrée I/O |
| Scènes | `scenes()`/`scenes_mut()`/`world_and_scenes()` | le manager seul mutateur d'états ; l'emprunt scindé fourni |
| Renderer | `renderer()`/`renderer_mut()` → `Option` | l'Option EST la frontière headless |
| Arrêt / pause / échelle | `request_exit`/`request_pause`/`request_resume`/`set_time_scale` | des REQUÊTES, appliquées à la frontière de frame — jamais d'effet immédiat |
| Défaillance fatale | `report_fatal` | l'escalade d'un subsystem qui ne peut pas continuer : diagnostic conservé, arrêt ordonné, `Err` par `Engine::run` (le modèle complet : section « Le modèle d'erreurs ») |
| Diagnostics | `diagnostics()` | le profil CPU de la dernière frame COMPLÈTE (`last_frame()`) + dépassements cumulés — lecture seule (la section « Les diagnostics et le profiling CPU ») |
| Metrics | `metrics()` | la santé synthétique et continue (`snapshot()` : FPS, jauges, compteurs, états des subsystems) — lecture seule, découplée de toute UI |

Anti-patrons bannis : l'état global caché (`static mut`, once_cell, thread_local — **verrouillé en CI**, `chaos_engine/tests/boundaries.rs`) ; la dépendance directe entre subsystems (le World est le canal, démontré par test). Le patron de test des subsystems : un `Engine` headless (`start()` + `on_update()` sans fenêtre). Les contraintes de threads de chaque service : `docs/architecture/threading.md`.

## Le cycle de vie des subsystems

**Enregistré** (`add_subsystem`) → désactivation par configuration (`runtime.disabled_subsystems`) → retrait des graphiques si headless (`requires_graphics()`) — retiré à l'une de ces étapes = jamais initialisé, jamais tické → tri par dépendances → initialisé → **Actif** (reçoit `on_event`/`update`/`render`, dans l'ordre trié) → **Arrêté** (`shutdown`, ordre INVERSE) → Détruit (drop).

- **Les dépendances se déclarent par NOM** (`Subsystem::dependencies`, vide par défaut) — couplage faible : des chaînes, jamais des types. Pas de conteneur DI généraliste : un tri et trois refus.
- **Le tri est topologique et DÉTERMINISTE** (Kahn stable : à égalité, l'ordre d'enregistrement départage) — sans dépendance déclarée, le tri est l'identité. L'ordre n'est plus une coïncidence : c'est un contrat testé.
- **Refus explicites au démarrage** (le moteur ne démarre pas, erreur remontée par `Engine::run`) : nom dupliqué, dépendance inconnue (« 'x' depends on 'y' which is not registered »), cycle (les participants nommés), désactivation d'un nom non enregistré (« cannot disable subsystem 'x' »).
- Un échec d'init n'arrête que les subsystems déjà initialisés, en ordre inverse TRIÉ.
- **La classification est déclarée, plus devinée** : `requires_graphics()` remplace la dormance comme mécanisme premier (en headless, le subsystem graphique est RETIRÉ — jamais un no-op silencieux) ; la garde interne (`context.renderer()` absent → no-op, le patron de `GeometryDemo`) reste une défense en profondeur. Une API d'activation individuelle viendra avec son besoin réel (l'éditeur).

## La machine d'états du moteur

```
Created ──start()──▶ Running ◀──────────┐
   │                    │   ▲           │
   │ (échec d'init)     │   └─resume────┤
   ▼                    ▼ pause         │
Stopped ◀──shutdown── Paused ───────────┘
```

| Transition | Déclencheur | Règles |
|---|---|---|
| `Created → Running` | `start()` (après init des subsystems) | la configuration est validée EN TÊTE (l'invalide n'atteint jamais l'init) ; `start()` hors `Created` = refus explicite (`error!`) ; toute requête de pause en attente est purgée |
| échec d'init | un subsystem échoue | seuls les subsystems DÉJÀ initialisés sont arrêtés (ordre inverse), l'erreur remonte par `Engine::run()` |
| `Running → Paused` | `context.request_pause()` | appliquée à la FRONTIÈRE de frame (point déterministe), `info!` |
| `Paused → Running` | `context.request_resume()` | idem ; l'horloge est RESYNCHRONISÉE à la reprise : delta quasi nul — aucun saut, aucune rafale de pas fixes ; la durée de la pause reste créditée à `real_elapsed` |
| requête hors état | — | écartée avec `debug!` — refus explicite, jamais un effet différé |
| `→ Stopped` | fin de boucle / exit | shutdown des subsystems en ordre inverse + nettoyage des scènes ; **idempotent** (répétition = no-op) ; update/rendu/événements ensuite ignorés |

**Sémantique de la pause** : la simulation gèle — pas de tick d'horloge (`Time` et `frame_index` figés), pas de schedule ECS, pas d'update de subsystems. Le RENDU continue (la fenêtre reste vivante, resize compris) et les ÉVÉNEMENTS circulent (`on_event` — c'est par eux que la reprise arrive : la démo utilise **P**). Les messages ECS sont balayés à chaque tick de pause (personne ne les consomme — les subsystems les reçoivent par `on_event`) : pas de croissance non bornée.

## Pause, suspension et focus

Le comportement face aux interruptions est EXPLICITE — qui continue, qui s'arrête, comment le temps est traité, comment les inputs sont purgés :

| Interruption | Simulation | Rendu | Temps | Inputs/messages |
|---|---|---|---|---|
| pause app (`request_pause`/P) | gelée | continue | gelé (ni tick ni pas fixes) | événements circulent, messages balayés |
| perte de focus (`runtime.pause_on_focus_loss`, défaut OFF) | pause AUTO si la politique est active | continue | gelé | `Focused(false)` atteint TOUS les subsystems — chacun purge son état tenu (le patron `DebugCameraController`) |
| minimisation | continue (sauf politique focus) | le backend suspend de lui-même (surface à taille nulle → `FrameSkipReason::ZeroArea`) | continue | — |
| suspension OS (`WindowEvent::Suspended`) | pause AUTO, inconditionnelle (l'OS l'exige) | les hooks `render` ne sont plus appelés | gelé | l'événement atteint tous les subsystems (purge) |
| reprise (focus retrouvé / `WindowEvent::Resumed`) | reprend SEULEMENT une pause AUTO — une pause demandée par l'app est respectée, jamais relancée par le moteur | reprend | **horloge RESYNCHRONISÉE : delta quasi nul, aucun saut, aucune rafale de pas fixes** ; la durée de la pause reste créditée à `real_elapsed` (la vérité murale) ; le clamp 250 ms reste le filet des gels involontaires (breakpoint, machine) | rien de fantôme ne traverse : les messages sont balayés pendant la pause, l'état tenu a été purgé à l'interruption |

- **Le contrat de purge des inputs** : le moteur ne possède AUCUN état d'input central — les subsystems possèdent leur état tenu (touches maintenues, drag). Le moteur GARANTIT que l'événement d'interruption (`Focused(false)`, `Suspended`) atteint chaque subsystem dans l'ordre trié : c'est le signal de purge, `DebugCameraController` montre le patron.
- **Anti-sauts à la reprise** (le checkpoint) : le clamp de `FrameClock` (250 ms max de temps de jeu), zéro pas fixe fantôme (aucune accumulation pendant la pause), messages balayés — après une interruption prolongée : ni touche fantôme, ni delta géant, ni état incohérent (verrouillé par tests).
- **La distinction pause app / pause auto** : le moteur ne relance QUE ses propres pauses (`auto_paused` interne) — presser P puis alt-tab-retour laisse le moteur en pause, comme demandé.
- La suspension OS est traduite par `chaos_window` (`suspended()`/`resumed()` de winit → `WindowEvent::{Suspended, Resumed}`) ; la recréation de surface mobile viendra avec la plateforme.

## Le trait `Subsystem` : la prise murale des phases futures

Renderer, scènes, ECS, physique, audio, réseau et runtime se brancheront via `Engine::add_subsystem` sans modifier la boucle :

```rust
pub trait Subsystem {
    fn name(&self) -> &str;
    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()>;
    fn on_event(&mut self, event: &Event, context: &mut EngineContext);
    fn update(&mut self, context: &mut EngineContext);
    fn render(&mut self, context: &mut EngineContext);
    fn shutdown(&mut self, context: &mut EngineContext);
}
```

`EngineContext` est la vue que le moteur offre aux subsystems : temps, demande d'arrêt, et les **services partagés** — à commencer par le Renderer (`context.renderer_mut()`, `None` hors fenêtre pour garder les subsystems testables sans GPU). C'est par ce canal que le contenu crée ses ressources GPU et soumet ses draws ; les futurs services (assets, etc.) suivront le même chemin.

## Temps et cadence

- `FrameClock` borne le delta (250 ms par défaut) : un gel — breakpoint, mise en veille — ne produit jamais de pas de temps géant.
- `EngineConfig::target_fps` (60 par défaut) fixe l'échéance de la prochaine frame ; la boucle attend via `ControlFlow::WaitUntil` — **jamais par un sleep sur le main thread**. Sur macOS, toute occupation du main thread (sleep ou present vsync bloquant) rend le déplacement/resize de fenêtre laggy d'environ une seconde (winit #1737) ; c'est pour la même raison que `EngineConfig::vsync` est désactivé par défaut tant que le rendu est trivial. `None` = boucle libre.
- `EngineConfig::frame_limit` arrête proprement le moteur après N frames — hook de test/CI (`CHAOS_FRAME_LIMIT` dans sandbox).

## Points d'accroche prévus (non implémentés)

- **Physique** : un pas de temps fixe (accumulateur) viendra compléter l'update à pas variable.
- **Runtime/plateforme** : `sandbox` dépend directement de `chaos_engine` pour le bring-up ; il rebasculera sur `chaos_runtime` quand la couche plateforme prendra vie.
- **Logging** : les crates n'utilisent que la façade `log` ; un passage à `tracing` ne toucherait que les consommateurs (apps).
