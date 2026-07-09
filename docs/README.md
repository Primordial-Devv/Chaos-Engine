# docs

Documentation technique interne, organisée par domaine : architecture, renderer, ecs, assets, modding, networking, editor, server, distribution.

Conventions transverses :

- `versioning.md` — versioning technique `MAJOR.MINOR.PATCH(+BUILD)` et nom public `Chaos N`.
- `architecture/math-conventions.md` — conventions mathématiques d'autorité : repère main droite, +Y haut, -Z avant, TRS, quaternions, MVP, NDC wgpu.
- `git-workflow.md` — modèle de branches (`main`/`dev`/branches de travail), cycle de PR et rôle de la CI.
- `testing.md` — tous les tests exécutables : unitaires, end-to-end sandbox, trace des événements, portes de qualité.
