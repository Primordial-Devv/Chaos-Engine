# Boucle moteur et cycle de vie

Référence des choix d'architecture de la phase 1 (fenêtre, événements, boucle, cycle de vie). Tout ce qui suit conditionne les phases futures.

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

## Cycle de vie du moteur

```
Engine::run()
  └─ run_event_loop()                        boucle OS démarrée
       ├─ on_window_ready(WindowHandle)      fenêtre native créée
       │    ├─ enregistrement du RenderSubsystem (en dernier)
       │    └─ init des subsystems           dans l'ordre d'enregistrement
       ├─ on_event(Event)                    système + entrées, traduits
       │    └─ CloseRequested → request_exit
       ├─ on_update()                        chaque frame (about_to_wait) :
       │    ├─ gating : rien avant l'échéance de frame (target_fps)
       │    ├─ FrameClock::tick()            delta borné (max 250 ms)
       │    ├─ update de chaque subsystem    phase simulation
       │    ├─ frame_limit éventuel
       │    └─ request_redraw()
       ├─ frame_deadline()                   → ControlFlow::WaitUntil(échéance)
       ├─ on_redraw()                        sur RedrawRequested :
       │    └─ render de chaque subsystem    phase présentation
       └─ on_shutdown()                      subsystems arrêtés en ordre INVERSE
```

La séparation update/render suit le modèle winit : la simulation vit dans
`about_to_wait`, la présentation dans `RedrawRequested` — ce qui garde le rendu
fluide pendant le resize interactif macOS (boucle modale). Détails du renderer :
`docs/renderer/overview.md`.

États : `Created → Running → Stopped`. Un échec d'init d'un subsystem interrompt le démarrage, n'arrête que les subsystems déjà initialisés (ordre inverse) et fait remonter l'erreur par `Engine::run()`.

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

- **Renderer** : `WindowHandle` exposera les raw handles nécessaires à la création de surface ; `request_redraw` existe déjà.
- **Physique** : un pas de temps fixe (accumulateur) viendra compléter l'update à pas variable.
- **Runtime/plateforme** : `sandbox` dépend directement de `chaos_engine` pour le bring-up ; il rebasculera sur `chaos_runtime` quand la couche plateforme prendra vie.
- **Logging** : les crates n'utilisent que la façade `log` ; un passage à `tracing` ne toucherait que les consommateurs (apps).
