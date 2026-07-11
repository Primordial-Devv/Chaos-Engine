# Validation finale — Rendering Core Consolidation (mature V1)

**Date de l'audit : 11 juillet 2026 · Machine de validation : Apple M4 Pro / Metal (macOS) · Chaos Engine 0.2.0, branche `feature/redererCoreConsolidation`.**

Ce document est l'INDEX DE PREUVES de la consolidation (sous-phases 1 à 18) : il pointe les tests, les checkpoints, les preuves vivantes de la démo et les sections de documentation — il ne les recopie pas. Les références de tests sont des noms exacts (`cargo test <nom>`) ; les sections renvoient à `docs/renderer/overview.md` (« overview ») et `docs/testing.md` (« testing »).

## 1. La matrice des domaines — chaque domaine, sa preuve

| Domaine | Checkpoint / tests | Preuve vivante (démo) | Doc |
|---|---|---|---|
| API publique | `#![deny(missing_docs)]` (build), sweep d'audit §2.8 | — | overview « La frontière publique » |
| Isolation backend | `wgpu_never_leaks_outside_the_backend`, `wgpu_dependency_lives_in_a_single_manifest` | — | overview « Garanties d'isolation » |
| Ressources GPU & durée de vie | `checkpoint` 100 cycles (durée de vie), `resources_survive_intensive_churn` | logs debug (créations nommées) | overview « La durée de vie des ressources » |
| Mémoire | `resource_stats` octets EXACTS ; `long_execution_never_drifts` (1 000 frames) ; long run réel §4 | `CHAOS_DIAG_FRAME` | testing 2quater |
| Textures & mipmaps | chaîne de mips à l'octet près, box filter, `update_texture` contrôlé (chunk « système de textures mature ») | damier mippé anisotrope du sol | overview « Textures » |
| Cubemaps & HDR | cubemap 6 faces validée, `Rgba16Float` binary16 exact, mips par face | `demo.sky` HDR | overview « L'environnement et le ciel » |
| Render targets & offscreen | checkpoint sous-phase 4 (rendre → échantillonner → resize → re-rendre), `render_target_rotation_is_leak_free` | l'écran de surveillance | overview « Render targets » |
| Passes | checkpoint sous-phase 5 (miroir ordre -10 + refus nommés) | `demo.mirror` | overview « L'orchestration des passes » |
| Materials | checkpoint sous-phase 6 (partage, modification in-place, contrats layout) | 31 materials descriptifs | overview « Materials » |
| Lighting | `checkpoint` Lighting V1 (toggle observable), verrou `MAX_LIGHTS` Rust↔WGSL | K, flaques orbitantes | overview « L'éclairage » |
| PBR | checkpoint grille metallic×roughness, 7 slots refcountés | grille 4×4, bumpy, émissive | overview « Le matériau PBR » |
| Environnement, ciel, IBL, exposition | checkpoint sous-phase 9 (deux passes, réglages sans rebind) | E, V/B | overview « L'environnement et le ciel » |
| Ombres | `checkpoint_shadows_v1_full_scene_over_two_frames` + verrous naga ombre | N, K — ombres stables | overview « Les ombres » |
| Transparence & masked | `checkpoint_transparency_v1_full_scene_over_two_frames` (ordre à quatre temps, tri suivant la caméra) | verres triés, grille masked | overview « L'opacité et l'ordre de rendu » |
| Instancing | `checkpoint_instancing_v1_a_crowd_collapses_to_a_few_draw_calls` (504 → 5) | l'essaim : 1 200 → 1 draw | overview « L'instancing » |
| Culling | `checkpoint_culling_v1_a_stress_scene_pays_only_for_the_visible` (1 001 → 100, l'anti-pop) | O : culled en vol | overview « La visibilité » |
| Debug rendering | `checkpoint_debug_v1_the_visual_language_lives_and_expires` + verrou naga debug | G/X/F/J/T | overview « Le debug rendering » |
| Diagnostics & profiling CPU/GPU | `checkpoint_diagnostics_v1_the_frame_explains_itself` ; gpu MESURÉ en réel (§4) | O : le snapshot | overview « Les diagnostics du renderer » |
| Fallbacks | `degraded_permutations_and_builtins_are_visible_fallbacks` ; caches `Option` mémoïsés | rapport de capacités | overview « La robustesse multiplateforme » |
| Multiplateforme | `checkpoint_robustness_v1_no_capability_is_implicit` ; refus device nommés | `gpu capabilities:` à l'init | overview « La robustesse multiplateforme », testing 2ter |
| Stress | la suite (12 tests, `cargo test -p chaos_renderer suite`) + `chaos_engine/tests/stress.rs` | longs runs §4 | testing 1bis |
| Validations visuelles | scène documentée + checklists interactives (HUMAINES, assumé) | sections 2/3 de testing | testing |
| Documentation | comptes re-vérifiés à l'audit (§2.9) | — | overview + testing alignés |

## 2. Les attestations d'architecture — vérifiées à l'audit

1. **Aucune dépendance ECS** — `cargo tree -p chaos_renderer` : chaos_core, log, pollster, raw-window-handle, wgpu (+ naga en dev). Verrou : `chaos_ecs/tests/isolation` (3 tests).
2. **Aucune dépendance Scene System** — même arbre. Verrou : `chaos_scene/tests/isolation` (3 tests).
3. **Aucune logique Asset Pipeline absorbée** — zéro `std::fs`/`std::io`/format de fichier dans `chaos_renderer/src` (grep d'audit : 0) ; la couture assets→renderer vit dans `chaos_engine::assets`.
4. **Aucune logique gameplay / éditeur** — grep d'audit : 0 ; l'API est descripteurs + handles + draws.
5. **Aucune API wgpu hors backend** — verrou CI `wgpu_never_leaks_outside_the_backend` (sources) + `wgpu_dependency_lives_in_a_single_manifest` (manifestes), re-passés à l'audit.
6. **Aucun accès backend depuis le sandbox** — verrou `applications_see_only_the_facade` (chaos_engine/tests/boundaries) ; grep sandbox : zéro référence (un seul commentaire de convention).
7. **Aucun global caché** — verrou `the_engine_has_no_hidden_globals` (workspace entier).
8. **Aucune abstraction sans consommateur** — sweep des 101 exports de `lib.rs` : 62 consommés hors crate ; 39 sans consommateur externe ACTUEL, tous classés dans les audiences déclarées : (a) le vocabulaire de l'IMPLÉMENTEUR DE BACKEND (les signatures de `GraphicsBackend` les consomment — `FramePlan`, `FrameDraw`, `PipelineDescriptor`, `MaterialParams`, `ShaderRef`, les handles…), (b) les COMPOSANTS d'agrégats consommés (les champs de `RendererDiagnostics` : `PassStats`, `CpuCost`, `SurfaceStats`…), (c) les bornes et conventions symétriques (`MIN/MAX_SHADOW_RESOLUTION`, `DEFAULT_DEBUG_CATEGORY`), (d) `Frustum` (l'outil déclaré du futur éditeur). **Point de veille** : `RenderQueue` est exporté sans consommateur externe ni signature publique qui l'exige — candidat à la re-privatisation ou au consommateur éditeur ; décision léguée (registre §3).
9. **Aucune fonctionnalité prématurée** — chaque sous-phase liste ses « extensions notées, pas construites » ; l'audit n'a trouvé aucun code mort ni chemin sans preuve vivante.
10. **Aucune régression Engine Core / Asset Pipeline / ECS / Scene System** — `cargo test --workspace` : 787 tests verts (135 engine, 74 assets, 100 ecs, 101 scene, 51 core, 4 window, 322 renderer) ; le run headless (l'application complète par l'API publique) inchangé.

## 3. Le registre de la dette V1 — des choix documentés, pas des oublis

Chaque item est une limite EXPLICITE d'une sous-phase, avec sa section de doc et son extension notée :

| Dette | Où c'est dit | Le remboursement prévu |
|---|---|---|
| Pipelines PERMANENTS (pas de destruction, handle non générationnel) | overview « Pipelines » | gestion mémoire des pipelines |
| Tri des transparents PAR OBJET | overview « L'opacité » | tri par triangle / OIT |
| Silhouette d'ombre PLEINE des masked | overview « Les ombres » | casters alpha-testés |
| UNE directionnelle projette ; volume statique ; PCF 3×3 fixe | overview « Les ombres » | cascades, fitting, ponctuelles |
| IBL approximé (pas de préfiltre GGX ni BRDF LUT) | overview « Ce que les phases futures brancheront » | le raffinement PBR |
| AABB par draw, pas de BVH ; pas d'occlusion culling | overview « La visibilité » | hiérarchies spatiales |
| Transparents jamais instancés ; slot objet écrit non lu | overview « L'instancing » | tri par instance, skip du slot |
| Lignes debug 1 px ; pas de texte 3D ; pas de handle par primitive | overview « Le debug rendering » | quads orientés caméra |
| Span GPU frame ENTIÈRE (pas par passe) ; latence de quelques frames | overview « Les diagnostics » | le query set est dimensionné pour |
| Défauts WebGPU seuls (pas d'élévation de limites) | overview « La robustesse » | l'élévation ciblée |
| `render_to_target` : pas d'ombre, pas de debug, tri depuis l'origine | overview (sections concernées) | la passe immédiate complète |
| Dédup de pipelines non parfaite (`None` ≠ format surface) | overview « Pipelines » | la clé au format résolu |
| Validation visuelle HUMAINE (pas de golden images) | testing 2quater | l'infra CI GPU |
| Dynamic offsets non consommés (slots dédiés par draw) | overview « La robustesse » | l'optimisation des uniforms |
| `RenderQueue` exporté sans consommateur externe | ce document §2.8 | re-privatiser ou consommer (éditeur) |
| `renderer.rs` 9 388 lignes (le banc d'essai — ~530 lignes — extrait en sous-phase 17) | ce document | le démontage en modules — décision AVANT l'éditeur |

## 4. Les runs finaux consignés (11 juillet 2026, M4 Pro/Metal)

- `cargo check --workspace --all-targets` ✓ · `cargo fmt --all --check` ✓ · `cargo clippy --workspace --all-targets --all-features -- -D warnings` ✓ · `cargo test --workspace` : **787 verts**.
- GPU 180 frames : code 0. Headless 2 400 frames : code 0.
- **Long run 1 800 frames** (`CHAOS_DIAG_FRAME=1700`) : mémoire IDENTIQUE À L'OCTET (17 640 716 o, 0 retraite) entre la frame 150 et la frame 1 700, mêmes 27 pipelines, mêmes 34/2/27 draw calls, `gpu: 1.80 ms` MESURÉ (timestamp queries Metal), `cpu: ~1.4 ms`, 1 697 présentées + 3 sautées à l'init — **zéro dérive**.
- Capacités à l'init : `gpu capabilities: all domains active (Metal on Apple M4 Pro)` — timestamps actifs, sRGB préféré offert, limites 8192 px / 256 Mio respectées.

## 5. Le checkpoint final du runbook — coché

Ombres V1 ✓ (matrice) · règles explicites opaque/masked/transparent ✓ · tri transparent cohérent ✓ (suit la caméra, stable) · l'instancing réduit RÉELLEMENT (504→5 en test ; 1 249→34 en démo) ✓ · le culling réduit RÉELLEMENT (1 001→100 en test ; mesurable à l'écran par O) ✓ · debug rendering utilisable ✓ (G/X/F/J/T) · diagnostics CPU/GPU exploitables ✓ (snapshot Display, gpu mesuré) · ressources maîtrisées ✓ (baseline prouvée au churn et au long run) · fallbacks explicites ✓ (capacités + FallbackStats) · macOS validé ✓ (runs réels continus) · Windows : checkpoint externe DÉCLARÉ ✓ (testing 2ter) · stress verts ✓ (suite 12 tests) · validations visuelles cohérentes ✓ (humaines, documentées) · wgpu confiné ✓ (verrous re-passés) · ni ECS, ni scènes, ni formats de fichiers ✓ (attestations 1–3) · les 4 portes cargo ✓ · runs GPU sans erreur ✓ · documentation à jour ✓ (comptes re-vérifiés).

## 6. Déclaration de maturité

**Le Rendering Core de Chaos Engine est déclaré MATURE V1.** Il est une dépendance STABLE pour Chaos Editor et les systèmes de Chaos Engine 1.0 :

- **l'API publique est un contrat** : descripteurs backend-agnostic, handles générationnels opaques, validation avant le backend, erreurs nommées — les deux audiences (consommateur / implémenteur de backend) sont documentées dans `lib.rs` ;
- **les invariants tiennent sous stress** : la mémoire revient à la baseline, les longs runs ne dérivent pas, les erreurs n'empoisonnent jamais la frame suivante, les capacités absentes se signalent au lieu de casser ;
- **ce qui n'est PAS garanti est ÉCRIT** : le registre de dette (§3) est la liste exhaustive des approximations V1 — chacune a sa doc et son chemin de remboursement.

La consolidation (sous-phases 1–18) est CLOSE.
