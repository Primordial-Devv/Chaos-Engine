# Versioning de Chaos Engine

Le moteur porte deux identités distinctes qui ne doivent jamais être confondues :

| Identité | Forme | Usage |
|---|---|---|
| **Version technique** | `MAJOR.MINOR.PATCH` (+`BUILD` optionnel) | Documentation, fichiers du moteur, logs, builds, outils de développement |
| **Nom public** | `Chaos N` | Launcher, communication, interface publique |

## Version technique — `MAJOR.MINOR.PATCH(+BUILD)`

Le schéma suit [SemVer](https://semver.org/lang/fr/) :

- **MAJOR** — nouvelle génération du moteur. Refonte ou rupture majeure : architecture, format de données, contrat de `chaos_api`.
- **MINOR** — évolution importante de la génération courante, sans rupture de compatibilité.
- **PATCH** — corrections, améliorations mineures, optimisations. Aucune modification d'architecture.
- **+BUILD** *(optionnel, interne uniquement)* — identifie précisément une compilation, une révision ou une version de développement (ex. numéro de CI, compteur de commits, hash court). Conformément à SemVer, la métadonnée de build est ignorée pour la précédence des versions : `1.2.5+189` et `1.2.5+200` sont la même version publiée.

Exemples valides :

```
1.0.0
1.2.0
1.2.5
1.2.5+189
```

### Règles

1. Le numéro de BUILD n'apparaît jamais dans un `Cargo.toml` ni dans un tag de release : il est injecté au moment de la compilation (outillage/CI, à venir).
2. Une version publiée ne se réutilise jamais ; toute correction passe par un nouveau PATCH.
3. Les tags git de release suivront la forme `vMAJOR.MINOR.PATCH`.

## Nom public — `Chaos N`

`N` correspond au **MAJOR** de la version technique. Le nom public ne porte jamais le détail MINOR/PATCH :

| Nom public | Versions techniques couvertes |
|---|---|
| Chaos 1 | `1.x.y` |
| Chaos 2 | `2.x.y` |
| Chaos 3 | `3.x.y` |

Le nom public est réservé à la couche visible (launcher, site, communication). Tout ce qui est technique — docs internes, logs, noms de builds, outils — utilise la version complète.

## Phase actuelle — `0.x.y`

Le workspace est en `0.1.0` : phase de construction de la première génération. Conformément à SemVer, tant que MAJOR vaut 0, les ruptures de compatibilité sont permises en MINOR. **Chaos 1** naîtra avec la publication de `1.0.0`.

## Source de vérité unique

La version technique vit à un seul endroit : `[workspace.package].version` dans le `Cargo.toml` racine. Les 14 crates et 3 apps en héritent via `version.workspace = true` — l'ensemble du moteur évolue en version unique (lockstep). Un bump de version = une seule ligne modifiée.

## Évolutions prévues (rien d'implémenté)

- `chaos_core` exposera les constantes de version du moteur (via `CARGO_PKG_VERSION`) pour les logs et les outils.
- L'outillage de build/CI injectera la métadonnée `+BUILD` dans les binaires de développement.
- Le contrat `chaos_api` recevra, le moment venu, son propre versionnement de compatibilité pour les mods — distinct de la version du moteur et documenté séparément.
