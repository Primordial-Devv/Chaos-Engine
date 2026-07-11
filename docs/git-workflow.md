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

Deux rulesets actifs (repo public), aux exigences adaptées au sens du flux :

| Ruleset | Cible | Règles |
|---|---|---|
| `protect-dev` | `dev` | PR obligatoire, checks `check`/`fmt`/`clippy` requis, **branche à jour exigée** (pertinent pour `feature → dev`), merges squash + commit |
| `protect-main` | `main` | PR obligatoire, checks requis, à-jour non exigé (les releases `dev → main` passent sans back-merge), **merge commit uniquement** |

Communs aux deux : force-push et suppression de branche bloqués, aucun acteur de bypass. La scission existe parce qu'un ruleset unique exigeant « à jour » partout auto-bloquait les releases : `main` porte des merge commits que `dev` ne peut pas rattraper, `dev` étant elle-même protégée.

## CI

Le workflow `.github/workflows/ci.yml` exécute `cargo check`, `cargo fmt --check` et `cargo clippy --all-targets --all-features -- -D warnings` sur **chaque push de n'importe quelle branche** (retour immédiat dès le premier push, avant même d'ouvrir une PR) et sur chaque PR vers `dev`/`main` (validation du résultat du merge). Quand une PR est ouverte, un push déclenche donc les deux — le repo étant public, les minutes Actions sont gratuites.
