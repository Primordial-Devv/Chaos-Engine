# Conventions mathématiques de Chaos Engine

Document d'autorité. Tout système du moteur — renderer, caméras, scènes, physique, animation, imports glTF, gizmos de l'éditeur, scripts, réseau — interprète l'espace selon ces règles, sans exception. Les constantes exécutables vivent dans `chaos_core::math::world` ; des tests numériques verrouillent chaque convention.

## Repère et axes

| Règle | Valeur |
|---|---|
| Repère | **Main droite** : `X × Y = Z` |
| +X | Droite |
| +Y | **Haut** |
| Avant (regard) | **-Z** (`math::world::FORWARD`) |
| Unités | 1 unité = **1 mètre** |
| Angles | **Radians**, partout, toujours |

## Matrices (glam)

- Stockage **column-major** (la translation d'une `Mat4` vit dans `w_axis`).
- **Post-multiplication** : `M * v` transforme le vecteur ; composition `parent * enfant`.
- Ordre des transformations : **TRS** — échelle, puis rotation, puis translation (`Mat4::from_scale_rotation_translation`, `Transform::matrix`).

## Rotations

- Représentation stockée : **quaternions** (`Quat`), jamais d'angles d'Euler persistés (Euler accepté en entrée utilitaire uniquement — `Quat::from_rotation_*`).
- Sens positif : **trigonométrique** (counterclockwise) autour de l'axe, vu depuis la pointe de l'axe — règle de la main droite. Exemple verrouillé : `from_rotation_y(π/2) * X = -Z`.

## Enroulement des faces (winding)

- Face avant = sommets en ordre **CCW (trigonométrique) vu de l'extérieur** de l'objet — cohérent avec le repère main droite et le défaut `FrontFace::Ccw` des pipelines.
- Conséquence : toute géométrie fermée bien enroulée supporte le **back-face culling** (activé sur les pipelines opaques standard) ; une géométrie 2D (triangle, quad) est **single-sided** sous un pipeline cullé — un pipeline double-sided (`CullMode::None`, le défaut du descripteur) la rend visible des deux côtés.
- Verrou : le test d'enroulement de `Geometry::cube` (chaque normale de triangle, par produit vectoriel, pointe vers l'extérieur du cube).

## Chaîne Model → View → Projection

```
clip = Projection × View × Model × vertex
```

- `Model` = `Transform::matrix()` de l'objet.
- `View` = **inverse** de la matrice monde de la caméra.
- Projections bénies : **`chaos_core::math::projection::*`** (point de passage unique, adossé aux projections main droite / profondeur 0..1 de glam). Le moteur n'appelle jamais les fonctions de projection de glam directement — les variantes OpenGL (-1..1) sont ainsi structurellement hors de portée, et un test verrouille la plage 0..1.

## Côté GPU (wgpu)

| Espace | Convention |
|---|---|
| NDC X, Y | [-1, 1], **+Y vers le haut** |
| NDC Z (profondeur) | **[0, 1]** — différent d'OpenGL |
| Framebuffer / UV | Origine **en haut à gauche** (s'appliquera à la phase textures) |

## Compatibilités en cascade

- **glTF** utilise nativement main droite / +Y haut / -Z avant → **imports sans conversion de repère**.
- Physique, réseau, scripts et éditeur héritent du même espace : une position sérialisée sur le réseau ou manipulée par un gizmo est la même donnée, sans adaptation.
