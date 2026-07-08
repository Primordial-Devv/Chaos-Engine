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

Le ruleset `protect-main-dev` est actif sur `main` et `dev` (repo public) :

- PR obligatoire — aucun push direct possible ; résolution des conversations exigée ; méthodes de merge autorisées : squash et merge commit.
- Checks `check`, `fmt`, `clippy` requis et branche à jour avec sa cible exigée avant merge.
- Force-push et suppression de branche bloqués. Aucun acteur de bypass.

En complément, le hook local `.githooks/pre-push` refuse les push directs vers `main`/`dev` avant même d'atteindre GitHub. À activer une fois par clone : `git config core.hooksPath .githooks`.

## CI

Le workflow `.github/workflows/ci.yml` exécute `cargo check`, `cargo fmt --check` et `cargo clippy --all-targets --all-features -- -D warnings` sur chaque PR vers `dev`/`main` et chaque push sur `dev`/`main`. L'événement PR re-testant chaque push de la branche de travail, aucun déclencheur supplémentaire n'est nécessaire sur les branches de travail.
