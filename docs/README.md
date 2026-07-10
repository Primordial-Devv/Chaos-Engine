# docs

Documentation technique interne, organisée par domaine : architecture, renderer, ecs, assets, modding, networking, editor, server, distribution.

Conventions transverses :

- `versioning.md` — versioning technique `MAJOR.MINOR.PATCH(+BUILD)` et nom public `Chaos N`.
- `architecture/math-conventions.md` — conventions mathématiques d'autorité : repère main droite, +Y haut, -Z avant, TRS, quaternions, MVP, NDC wgpu.
- `renderer/lighting-preparation.md` — le plan d'accueil vérifié de Lighting V1 et Material PBR (rien d'implémenté : la carte des points d'atterrissage).
- `assets/overview.md` — l'Asset Pipeline : le producteur des ressources (identité stable `AssetId`, noms logiques, règles de la phase 3).
- `ecs/overview.md` — l'ECS : le cœur logique, sous-système STABLE (entités générationnelles, composants en sparse sets, World, ressources, systèmes, scheduler à stages, requêtes, messages et commandes différées, intégration moteur).
- `scene/overview.md` — le Scene System : sous-système STABLE, la couche de structure et de persistance au-dessus de l'ECS (identités stables, appartenance par composant, hiérarchie, propagation des transforms, SceneManager multi-couches, format `.cscn` versionné, save/load par le pipeline, prefabs, frontière runtime) — la fondation de Chaos Editor.
- `git-workflow.md` — modèle de branches (`main`/`dev`/branches de travail), cycle de PR et rôle de la CI.
- `testing.md` — tous les tests exécutables : unitaires, end-to-end sandbox, trace des événements, portes de qualité.
