# Préparation du Lighting et du Material PBR — le plan d'accueil

Rien de ce document n'est implémenté. C'est le **plan d'atterrissage vérifié** de Lighting V1 et de Material PBR dans l'architecture de Rendering Core V3 : chaque besoin futur est mappé sur son point d'accueil dans le code réel, avec ce qui changera (mécanique) et ce qui ne changera pas (les invariants). Ces phases suivront cette carte — aucune refonte n'est nécessaire.

## La carte d'atterrissage

| Besoin Lighting V1 | Point d'atterrissage | Ce qui existe déjà | Ce qui changera (mécanique, pas refonte) |
|---|---|---|---|
| **Normales** | attribut de vertex | `VertexAttributeFormat::Float32x3`, `VertexLayout::packed` ; deux vertex standard (`ColorVertex`, `TexturedVertex`) prouvent l'extension ; les constructeurs `cube` calculent déjà la normale de chaque face (jetée aujourd'hui) | `LitVertex { position, normal, uv }` = troisième vertex standard, ou unification des géométries (déjà notée pour l'asset pipeline) |
| **Light uniforms** | `@group(0)` — fréquence frame | la convention `shaders::inputs` est l'autorité unique extensible ; le verrou naga grandit avec elle | `FrameUniforms` s'étend (position caméra pour le spéculaire) ; `LIGHTS_BINDING` s'ajoute aux constantes ; `Uniforms` (backend) gagne une entrée de layout |
| **Light buffers** (N lumières) | même groupe, buffer dédié | `BindingType::Buffer` uniform en place ; le passage aux storage buffers est une extension wgpu triviale | `set_view_projection` évoluera vers un état de frame plus riche — `FramePlan` porte déjà le « quoi » de la frame |
| **Normal matrix** | `@group(1)` — par objet | slots d'objet génériques réutilisés par index, `mat4_to_bytes`, `Transform::matrix()` + `Mat4::inverse` dans le choke point `chaos_core::math` | `ObjectUniforms` 64 → 128 octets (une constante + le write path) |
| **Shadow maps** | render-to-texture, compare sampler, pré-passes | `TextureUsage::RenderTarget` dans le vocabulaire ; `SamplerDescriptor.compare` noté extension depuis V3.5 ; la passe (`chaos.main_pass`) est un détail backend invisible de l'abstraction | format profondeur-cible dans `TextureFormat`, variante compare du sampler, `FramePlan` gagne des passes — le trait `GraphicsBackend::render(&FramePlan)` survit |
| **Environment maps** | cubemap au groupe frame ou material | extension kind/couches du `TextureDescriptor` notée depuis V3.1 ; `TextureDescriptor::validate()` est le point d'ancrage des règles (6 faces) | variante de vue cubemap côté backend, binding dédié dans `inputs` |
| **Paramètres PBR** (metallic, roughness…) | `MaterialUniforms` | le buffer material existe (binding 2 du groupe material), le descripteur a ses builders | 16 octets → la taille nécessaire (une constante backend + champs `with_*` du descripteur) |

## Material PBR — le plan d'évolution

Material V1 (pipeline + base_color + texture/sampler optionnels) évolue vers le material PBR **par addition**, jamais par rupture :

| Besoin PBR | État / évolution |
|---|---|
| **Base color** | ✅ existe — `MaterialUniforms.base_color`, `with_base_color()` |
| **Metallic / Roughness** (scalaires) | `MaterialUniforms` 16 → 32 octets (une constante backend + `with_metallic`/`with_roughness`) — cartographié dans la table lighting ci-dessus |
| **Normal map / AO / Emissive** (textures) | le layout du groupe(2) grandit en **slots fixes toujours remplis** : le patron fallback prouvé s'étend — `chaos.white` (albedo/AO neutres, existe), `chaos.normal` (normale plate `(128, 128, 255)`, linéaire), `chaos.black` (émissif éteint), tous servis par le cache de textures avec leurs clés prêtes. Descripteur additif : `with_normal_map`/`with_ao`/`with_emissive` |
| **Emissive couleur** | paramètre `MaterialUniforms` + texture optionnelle, même patron que base_color |
| **IBL** | environment maps (table lighting) + BRDF LUT = texture 2D standard, rien de nouveau |

Les deux garanties de non-blocage, vérifiées :

1. **Slots fixes toujours remplis** : chaque texture PBR optionnelle a son fallback neutre — un material minimal (couleur seule) reste un bind group complet et valide, exactement comme aujourd'hui avec `chaos.white`.
2. **Les shaders existants survivent au layout élargi** : en WGSL/wgpu, un shader qui ne déclare pas un binding présent dans le layout reste valide — `chaos.textured` fonctionnera sans modification sous le layout PBR. Le PBR sera un nouveau builtin (`chaos.pbr`), accueilli par la `ShaderLibrary` et le verrou naga des conventions.

Ce qui reste stable : `MaterialHandle`, le triplet `DrawCommand { mesh, material, transform }`, `create_material` pour tout le contenu existant. La règle sRGB est déjà verrouillée pour le PBR : albedo/emissive en sRGB, normal/metallic-roughness/AO en linéaire — le vocabulaire `TextureFormat` couvre les deux familles depuis V3.1. La mise à jour live des paramètres (tuning) viendra avec ses besoins réels.

## Les invariants qui garantissent « sans refonte »

1. **`shaders::inputs` est l'autorité unique** des groupes/slots : elle grandit (nouvelles constantes), elle ne casse pas — et le verrou naga suit.
2. **Le triplet `DrawCommand { mesh, material, transform }` est stable** : le lighting enrichit ce que les draws produisent à l'écran, pas leur forme.
3. **Le trait `GraphicsBackend` s'étend, ne casse pas** — précédent : six extensions pendant V3 (textures, samplers, material bindings), zéro rupture.
4. **wgpu reste confiné** : les mécaniques lighting (passes d'ombre, vues cubemap, buffers de lumières) seront backend-internes, exactement comme la profondeur l'a été (V2.6 : occlusion correcte sans toucher un type public).
5. **Les descripteurs ont des builders** : ajouter un champ n'a jamais cassé un constructeur existant.

## Ce qui n'a volontairement PAS été fait

Un attribut normal inutilisé, des constantes de bindings sans consommateur, un champ mips/kind figé à une seule valeur, un groupe de binding réservé mais vide : chacun de ces ajouts aurait été du **vocabulaire mort** — interdit par la règle « toute abstraction répond à un besoin réel ». La préparation du lighting n'est pas du code dormant : c'est la forme de l'architecture, plus cette carte.
