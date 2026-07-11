# Préparation du Lighting et du Material PBR — le plan d'accueil

**Lighting V1 a ATTERRI (consolidation, sous-phase 7) en suivant cette carte, sans refonte** — les quatre premières lignes sont réalisées telles qu'écrites : `LitVertex`, `FRAME_LIGHTS_BINDING` au groupe(0), le buffer de lumières uniform (16 max), `ObjectUniforms` 64 → 128 (normal matrix). Le PBR a suivi (sous-phase 8), puis les environment maps (sous-phase 9), puis les OMBRES (sous-phase 10) — la carte est intégralement réalisée.

## La carte d'atterrissage

| Besoin Lighting V1 | Point d'atterrissage | État |
|---|---|---|
| **Normales** | attribut de vertex | ✅ ATTERRI — `LitVertex { position, normal, uv }` (stride 32), `LitGeometry` (les constructeurs cube ont cessé de jeter leurs normales de face), chaîne d'assets complète (glTF `NORMAL` optionnel, synthèse plate à la couture `lit_geometry`) |
| **Light uniforms** | `@group(0)` — fréquence frame | ✅ ATTERRI — `FRAME_LIGHTS_BINDING = 1`, le verrou naga étendu ; la position caméra (spéculaire) reste pour le PBR |
| **Light buffers** (N lumières) | même groupe, buffer dédié | ✅ ATTERRI — uniform 1 056 o, `MAX_LIGHTS = 16`, troncature prévisible ; le passage aux storage buffers reste l'extension notée |
| **Normal matrix** | `@group(1)` — par objet | ✅ ATTERRI — `chaos_core::math::normal_matrix` (inverse-transposée, singulier → identité), `ObjectUniforms` 128 octets |
| **Shadow maps** | render-to-texture, compare sampler, pré-passes | ✅ ATTERRI (sous-phase 10) — en BACKEND-INTERNE, conformément à l'invariant 4 ci-dessous (le croquis initial « format profondeur public + passe déclarée » a cédé à son propre principe : la map, le sampler de comparaison et la passe d'ombre sont des organes internes, comme la profondeur l'a été en V2.6) : `FramePlan.shadow` dérivée par le renderer, `GraphicsBackend::set_shadow`, bindings 4–5 du groupe frame, `LightsUniforms` 1 056 → 1 136 (queue ombre) | |
| **Environment maps** | cubemap au groupe frame | ✅ ATTERRI (sous-phase 9) — la vue Cube côté backend, les bindings 2–3 du groupe frame dans `inputs`, le cube fallback noir interne, `set_environment`/`clear_environment`, le ciel `chaos.sky` et l'IBL V1 dans `chaos.pbr` | |
| **Paramètres PBR** (metallic, roughness…) | `MaterialUniforms` | le buffer material existe (binding 2 du groupe material), le descripteur a ses builders | 16 octets → la taille nécessaire (une constante backend + champs `with_*` du descripteur) |

## Material PBR — ATTERRI (consolidation, sous-phase 8)

**Le plan a été suivi à la lettre, par addition** : `MaterialModel::Pbr` + builtin `chaos.pbr` (Cook-Torrance GGX, layout `LitVertex`, tangentes dérivées à l'écran) ; `MaterialUniforms` 16 → 48 octets (base_color + metallic/roughness + émissif) mis à jour in-place (`set_material_metallic`/`roughness`/`emissive`) ; le groupe(2) élargi en **7 slots fixes toujours remplis** (base, sampler, uniforms, metallic-roughness, normal map, occlusion, émissif — fallbacks `chaos.white`/`chaos.normal_flat`) ; les shaders existants ont survécu au layout élargi comme promis ; `MaterialHandle` et le triplet `DrawCommand` intacts ; les conventions sRGB/linéaire et le packing glTF documentés dans la section « Le matériau PBR » d'overview.md. La position caméra (spéculaire) est entrée dans `FrameUniforms` (80 octets).

| Besoin PBR | État |
|---|---|
| Modèles éclairés, base color, metallic/roughness, normal map, AO, émissif | ✅ ATTERRIS |
| **IBL** | ✅ ATTERRIE (sous-phase 9) — cubemap d'environnement au groupe frame, mips box + BRDF analytique de Karis (approximations V1 documentées) ; RESTENT le préfiltre GGX et la BRDF LUT |
| **Import des materials glTF** | RESTE — attend les décodeurs d'images (PNG/JPEG) ; les conventions sont déjà alignées |

## Les invariants qui garantissent « sans refonte »

1. **`shaders::inputs` est l'autorité unique** des groupes/slots : elle grandit (nouvelles constantes), elle ne casse pas — et le verrou naga suit.
2. **Le triplet `DrawCommand { mesh, material, transform }` est stable** : le lighting enrichit ce que les draws produisent à l'écran, pas leur forme.
3. **Le trait `GraphicsBackend` s'étend, ne casse pas** — précédent : six extensions pendant V3 (textures, samplers, material bindings), zéro rupture.
4. **wgpu reste confiné** : les mécaniques lighting (passes d'ombre, vues cubemap, buffers de lumières) seront backend-internes, exactement comme la profondeur l'a été (V2.6 : occlusion correcte sans toucher un type public).
5. **Les descripteurs ont des builders** : ajouter un champ n'a jamais cassé un constructeur existant.

## Ce qui n'a volontairement PAS été fait

Un attribut normal inutilisé, des constantes de bindings sans consommateur, un champ mips/kind figé à une seule valeur, un groupe de binding réservé mais vide : chacun de ces ajouts aurait été du **vocabulaire mort** — interdit par la règle « toute abstraction répond à un besoin réel ». La préparation du lighting n'est pas du code dormant : c'est la forme de l'architecture, plus cette carte.
