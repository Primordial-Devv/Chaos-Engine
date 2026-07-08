# Workflow Git

## Branches

| Branche | Rôle | Push direct |
|---|---|---|
| `main` | Stable, production-ready, référence | Interdit |
| `dev` | Intégration, test, stabilisation | Interdit |
| `feature/…` `fix/…` `refactor/…` `docs/…` `chore/…` | Branches de travail temporaires, créées depuis `dev` | Oui (le temps de la PR) |

## Cycle de travail

1. Partir de `dev` à jour : `git switch dev && git pull`, puis `git switch -c feature/nom-court`.
2. Committer et pousser la branche, ouvrir une PR vers `dev`.
3. La CI (`check`, `fmt`, `clippy`) doit être verte et la branche à jour avec `dev`.
4. Merge en **squash** → un commit propre par PR dans `dev`. La branche de travail est supprimée automatiquement après merge.
5. Quand `dev` est stable : PR de `dev` vers `main`, mergée en **merge commit** (préserve l'ascendance commune entre les deux branches — squasher une release les ferait diverger).

## Règles

- `main` ne reçoit que du code validé, propre et stable, exclusivement via PR depuis `dev`.
- Jamais de force-push ni de suppression sur `main` et `dev`.
- Rebase merge désactivé au niveau du repo.
- Une PR = un sujet. Les branches de travail sont jetables et de courte durée.

## Protections GitHub

Le repo étant privé sur un plan GitHub Free, GitHub refuse l'activation des rulesets/protections de branches (réservés aux repos publics ou aux plans Pro et supérieurs). Les règles de ce document valent donc discipline d'équipe, renforcée par deux garde-fous :

- Hook local `.githooks/pre-push` qui refuse tout push direct vers `main` et `dev`. À activer une fois par clone : `git config core.hooksPath .githooks`.
- La CI s'affiche sur chaque PR ; ne jamais merger une PR dont les checks sont rouges.

Le jour où le repo passe sur GitHub Pro (ou devient public), activer le ruleset complet : PR obligatoire, checks `check`/`fmt`/`clippy` requis, branche à jour exigée, force-push et suppressions bloqués sur `main` + `dev` — via Settings → Rules → Rulesets, ou en relançant la création API (`gh api -X POST repos/…/rulesets`).

## CI

Le workflow `.github/workflows/ci.yml` exécute `cargo check`, `cargo fmt --check` et `cargo clippy --all-targets --all-features -- -D warnings` sur chaque PR vers `dev`/`main` et chaque push sur `dev`/`main`. L'événement PR re-testant chaque push de la branche de travail, aucun déclencheur supplémentaire n'est nécessaire sur les branches de travail.
