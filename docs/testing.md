# Tests du moteur

**Le Rendering Core est MATURE V1** — l'audit final de la consolidation
(matrice des preuves, attestations, registre de dette, runs consignés)
vit dans `docs/renderer/consolidation-validation.md`.

Tout se lance depuis la racine du workspace. Ce document couvre les phases 1 (cycle de vie, fenêtre, événements, boucle) et 2 (renderer minimal), Rendering Core V1, V2 et V3 (pipelines, géométrie, mesh, transforms, caméra, depth, RenderQueue, textures, samplers, bindings, materials, cache), la phase 3 Asset Pipeline (identité, registre, manager, importeurs, cache, validation, hot reload préparé) et la phase 4 ECS Core (entités, composants, world, ressources, systèmes, scheduler, requêtes, messages, commandes, intégration moteur) — et s'étoffera à chaque phase.

## 1. Tests unitaires

```sh
cargo test --workspace
```

| Crate | Tests | Ce qui est vérifié |
|---|---|---|
| `chaos_core` | 51 | Horloge de frame (temps réel vs jeu, échelle assainie, real_delta jamais clampé, scale 0, **resync de reprise** : l'écart avalé sans saut de delta, crédité au temps réel), **FixedClock** (pas exacts par accumulateur, cap anti-spirale avec excédent abandonné, FixedTime déterministe, mêmes deltas → mêmes séquences), Color, **Transform** (matrice, TRS, directions locales), **GlobalTransform** (identité, racine = matrice locale), **conventions mathématiques verrouillées** (main droite, column-major, rotations, profondeur 0..1 — la perspective ET l'orthographique bénies, `view::look_to` face au FORWARD béni), **normal_matrix** (la perpendicularité conservée sous échelle non uniforme, matrice singulière → identité — jamais des NaN), **Aabb** (`from_points` refusant vide et non-fini — les bounds invalides n'existent jamais —, `transformed` d'Arvo : translation exacte, rotation CONSERVATIVE), **Camera** (view inverse du transform, projection NDC centrée, composition P×V, aspect au viewport), **AssetId** (identité déterministe, algorithme FNV-1a verrouillé par vecteurs de référence, from_raw/value rebouclant pour la sérialisation), **Entity** (identité générationnelle, Display stable), **SceneId** (identité de scène dérivée du nom logique, même FNV-1a partagé, verrouillé par ses propres vecteurs, Display stable) |
| `chaos_ecs` | 100 | **Entities** (l'allocateur) : spawn/despawn, recyclage de slot avec génération neuve, entité périmée/forgée rejetée avec erreur explicite, double despawn en erreur, itération des vivantes seulement ; **ComponentStorage** (sparse set) : roundtrip avec Transform réel, remplacement rendant l'ancienne valeur, invariant swap_remove (les autres entités restent résolues), entité périmée jamais résolue, itération dense (Entity, &T), mutation en place ; **World** (le conteneur central) : garantie de vivacité à l'écriture (insert sur mort = erreur explicite nommant le type), despawn détachant tous les composants (index recyclé propre, insert ne déloge rien), despawn périmé sans effet sur l'occupant actuel, types multiples coexistant, valeurs indépendantes entre entités, `World: Send + Sync` verrouillé à la compilation ; **Resources** (les globales sans entité) : roundtrip avec `Time` réel, remplacement rendant l'ancienne valeur, retrait rendant la valeur, coexistence de types indépendants, survie au cycle de vie des entités, registre prouvé étanche aux composants (un même type dans les deux, sans collision) ; **Systems** (les traitements) : exécution dans l'ordre d'enregistrement (trace exacte), système réaliste (Time lu, Transforms avancés du delta), lecture/écriture de ressources, échec arrêtant le tick en nommant le système (mutations antérieures conservées), transformation de la structure du monde (spawn/despawn), doublon de nom rejeté ; **Schedule** (l'ordonnancement) : l'ordre vient des stages et non des appels (enregistrement croisé), ordre intra-stage conservé, même nom légal dans deux stages, doublon intra-stage rejeté, stage inconnu en erreur explicite, échec arrêtant tout en nommant stage + système, exécution stage par stage (un seul stage par index, noms par position déclarée, index hors bornes explicite — le mécanisme de l'instrumentation moteur) ; **Requêtes** : 10 000 entités mais seules les correspondances visitées, storage absent → vide, jointure exacte (les deux composants requis), sondage filtrant, mutation du meneur par la sonde, `query2_mut::<T, T>` en erreur explicite, composition des lectures pour jointures larges, corps de système sur requêtes de bout en bout ; **Messages** : FIFO exact, files auto-créées et indépendantes par type, drain consommant, clear balayant toutes les files, `chaos_core::Event` réel en roundtrip, flux complet via le World ; **Commands** : rien ne change avant l'apply, FIFO, despawns enregistrés pendant une requête puis appliqués, échec strict nommant l'index (file consommée, mutations antérieures conservées), patron flush cross-système via la ressource `Commands` ; **isolation** (tests d'intégration) : le renderer et l'Asset Pipeline ne mentionnent jamais chaos_ecs (sources + manifestes), chaos_ecs ne dépend que de chaos_core |
| `chaos_scene` | 101 | **Scene** (le modèle) : scène neuve Empty/identifiée/nommée, identité stable entre instances, les cinq états du cycle de vie distincts, détruire une scène ne touche jamais le World (la frontière de possession, prouvée) ; **appartenance** (`SceneMember`, un composant — jamais une liste) : spawn de membres vivants, members ne listant que SA scène, contains distinguant membre/globale/autre scène, adopt (revendication d'une globale, re-domiciliation rendant l'ancienne scène, entité morte en erreur explicite), unload despawnant tous ses membres et eux seuls (globales et autres scènes intactes, état Empty, réutilisable), membre despawné jamais attardé (référence périmée impossible par construction), composition avec les Commands différées, unload d'une scène vide en no-op ; **hiérarchie** (`ChildOf`, un lien jamais deux listes) : arbre construit et navigué (parent_of/children_of exacts), re-attach rendant l'ancien parent, soi-même/cycle/parent mort/enfant mort en erreurs explicites, detach rendant l'ex-parent puis None, enfants d'un parent despawné directement lus comme racines (auto-cicatrisation), despawn_recursive emportant le sous-arbre entier (compte exact, l'étranger survit), unload composant avec la hiérarchie (le global survivant se lit racine), composition avec les Commands ; **propagation** (`TransformPropagation`) : racine = local, enfant composé au parent, profondeur avec rotation, ancêtre sans Transform = identité, enfant d'un mort propagé racine, global balayé quand le Transform part, déplacer le parent déplace les descendants ; **conservation par opération** : attach préserve le local, attach_keeping_global/detach_keeping_global préservent le monde (local recalculé exact), sans Transform enfant == attach ; **SceneManager** (le point d'entrée unique) : create/register (doublon nommé), load peuplant via une source (Empty exigé, inconnu en erreur), chargement en échec → Failed avec contenu partiel (spawn/adopt refusés, unload = récupération), activate exigeant Loaded (déjà active = erreur), l'active indéchargeable directement (couches comprises), replace basculant ET détruisant l'état de la précédente, shutdown déchargeant tout en ordre trié, Send+Sync ; **multi-scènes** (les couches) : deux scènes actives indépendamment (isolation prouvée, deactivate/unload de l'une sans toucher l'autre), la première activée est la principale, désactiver la principale promeut la suivante, replace ne remplaçant QUE la principale (les couches intactes), replace sans principale = activation en tête, aucune active = état légitime ; **persistance** : release préservant à travers l'unload, release d'une non-membre en erreur explicite (pas de vol inter-scènes), release→adopt = préserver puis transférer, composant inconnu rejeté (directive inconnue nommée), déchargement répété inoffensif ; **isolation** (tests d'intégration) : le renderer et l'Asset Pipeline ne mentionnent jamais chaos_scene (sources + manifestes), chaos_scene ne dépend que de chaos_core et chaos_ecs ; **format de sérialisation** (`SceneData`) : capture déterministe (deux captures égales), membres/transforms/hiérarchie en indices de snapshot, parent hors snapshot capturé racine, **roundtrip sans perte** (capture → apply dans un monde frais → re-capture ÉGALE), membre sans Transform, apply composant avec le manager (la source populate réelle), validation rejetant version inconnue/parent hors bornes/auto-parenté/cycles/transforms non finis, apply validant d'abord (monde intact sur données invalides), MeshRef capturé/restauré, **encode→decode texte rebouclant bit-exact** (flottants irrationnels, quat tourné), encode déterministe, malformations nommées (en-tête, name, compte de flottants, hex, champ hors entité/ordre violé/directive inconnue) ; **Prefab** (la fondation) : capture parents-avant-enfants (liens externes coupés, racine morte en erreur), deux instanciations aux entités entièrement disjointes et hiérarchies parallèles, zéro état partagé entre instances, composants/hiérarchie restaurés à l'exact, instances membres de la scène cible (unload les emporte toutes), validation (vide, racine déplacée, racines multiples), placement par la racine propagé aux descendants |
| `chaos_window` | 4 | Traduction winit → types maison : touches, boutons, états, fallback `Unknown` |
| `chaos_assets` | 74 | **AssetRegistry** : enregistrement (id documenté, doublon rejeté en nommant l'existant), lookup nom → id, listage, transitions d'état (loaded/failed/unloaded, id inconnu rejeté) ; **AssetManager** : cycle de vie complet sur fichiers temporaires réels (roundtrip, cache prouvé par suppression du fichier, échec I/O → état Failed consultable, procédural non chargeable, unload idempotent + rechargement) ; **importeurs** : PPM P6 (décodage RGBA exact, commentaires, malformations nommées), WGSL (UTF-8), **glTF** (GLB construit octet par octet en test : positions/UV/indices exacts, UV zéros, l'accessor NORMAL lu quand présent et VIDE sinon — jamais des zéros dégénérés —, séquence d'indices générée, non-TRIANGLES et buffers externes rejetés, octets corrompus nommés ; `.gltf` auto-suffisant via data URI base64, décodeur verrouillé par vecteurs RFC 4648), import de bout en bout, routage kind+extension, importeur custom enregistré, importeur manquant → Failed ; **durée de vie** : acquire/release (mutualisation prouvée), unload protégé par la rétention, évincement des non-retenus, cycle streaming acquire → release → evict → reload ; **porte de validation** : règles sémantiques unitaires (indices hors bornes, NaN, désappariements — normales comprises : appariées si présentes, finies —, dimensions nulles) + importeurs malveillants refusés à la porte avec état Failed ; **hot reload (primitives)** : version de contenu (+1 par matérialisation, insensible aux échecs), reload sous rétention, donnée précédente conservée quand le nouveau fichier est invalide ; **jauges santé** : `loaded_count` suivant les transitions d'état, `cached_bytes` suivant le cache brut ; **fermeture** (`shutdown` : tout fermé MÊME sous rétention — caches vidés, états `Unloaded`, déclarations conservées —, idempotente) ; **politique de threads** : `AssetManager: Send + Sync` verrouillé à la compilation (la porte du chargement asynchrone — `AssetImporter: Send + Sync` par contrat) |
| `chaos_renderer` | 311 | Orchestration via backend factice (plan de frame, outcomes, pipelines, shaders, buffers, **textures** : forward du descripteur, validation dimensions/format/pixels portée par le descripteur (`validate()`) et appliquée avant le backend, **samplers** (défauts Linear+Repeat, builders), **bindings** (texture+sampler forward, binding par draw dans le plan), meshes, **uniforms** : view-projection dans le plan, Transform → matrice modèle par draw), géométrie (dont le **cube** : enroulement CCW verrouillé, couleur par face), **RenderQueue** (tri stable par pipeline, `draw_count` de la frame soumise), **vertex layouts déclaratifs**, **pool générationnel**, + **politique de threads** (`Renderer: Send` verrouillé à la compilation — la porte du futur render thread, `GraphicsBackend: Send` par contrat) + **durée de vie des ressources** (détruire une texture/un sampler partagé par des materials est REFUSÉ en nommant les dépendants, le buffer d'un mesh est protégé — « destroy the mesh instead » —, les fallbacks builtin sont indestructibles, le partage est compté consommateur par consommateur, la libération backend est DIFFÉRÉE à la fin de la frame suivante — prouvé par le journal factice —, les retraites s'observent puis se vident (`stats.retired`), les octets buffers/textures sont comptés EXACTS, les stats reviennent au niveau de base après destruction — zéro fuite logique —, double destruction = erreur périmée explicite, pipelines comptés permanents, et **le checkpoint : 100 cycles intensifs create/partage/destroy sans fuite ni handle périmé résolu**) + **système de textures mature** (chaîne de mips validée à l'octet près — niveau erroné nommé, trop de niveaux refusé —, cubemap carré à 6 faces validé, cube/mips interdits sur render target, `Rgba16Float` compté 8 o/px, `rgba16f_bytes_of` verrouillé sur valeurs binary16 connues, box filter aux moyennes exactes, mips fournis et cubemaps transmis au backend — `Generate` résolu AVANT lui —, `update_texture` contrôlé (octets exacts, mono-niveau 2D seulement, fallback et périmé refusés), material sur cubemap refusé clair, les trois builtins lazy/partagés/protégés, stats comptant la chaîne complète, anisotropie validée avant le backend — bornes et règle tout-Linear) + **render targets & rendu hors écran** (descripteur validé — dimensions nulles refusées —, création journalisée au backend et comptée aux stats — profondeur côté cible, couleur côté textures —, la couleur d'une cible alimente un material — l'entrée de passe —, `render_to_target` trie et résout par le chemin COMMUN — destination `target{i}` dans le journal, RenderQueue principale intacte, draws périmés écartés pareil, fonctionne surface suspendue (fenêtre minimisée) —, handle périmé = erreur explicite sur toutes les opérations — double destroy compris —, resize = ROTATION générationnelle — nouveau handle, l'ancienne couleur périmée, le material qui la tenait → draw écarté, retraite vidée à la frame suivante —, destroy REFUSÉ tant qu'un material partage la couleur — accepté après —, `with_color_target` transmis au backend, les stats revenant au niveau de base, et **le checkpoint : rendre hors écran → échantillonner la couleur dans la scène → resize → re-résolution → re-rendu, sans un type backend visible**) + **orchestration des passes** (la passe principale `chaos.main` existe à la construction — les tests exacts historiques la verrouillent au format près —, l'ORDRE vient du tri stable (order puis enregistrement) : prouvé par le journal sur deux frames identiques, labels validés — vide, dupliqué nommé, préfixe `chaos.` réservé —, dépendances invalides refusées en nommant écrivaine/lectrice/cible (« schedule it earlier »), lecture sans écrivain la même frame légale, `update_pass` revalidant TOUT le registre (repousser une écrivaine casse l'invariant entre deux passes tierces → refus, état intact), la passe principale protégée (destination/label refusés, load/caméra/ordre libres), passe désactivée absente du journal et `Disabled` au rapport puis réapparaissant, une file PAR passe (routage, `clear_draws` balaie tout, `draw_count` = la somme, handle inconnu explicite), une caméra PAR passe (deux vp distinctes au journal — un submit par passe), cible périmée au frame-time → AUTO-DÉSACTIVATION avec warn unique + `StaleTarget` au rapport + `update_pass` rebranchant le handle frais (le flux resize complet), feedback non déclaré (material échantillonnant la destination de SA passe) écarté du plan, plan vide (main off) court-circuitant le backend mais vidant la retraite, passes cible EXÉCUTÉES quand la surface est sautée (outcome conservé, rapport `Executed`/`SurfaceSkipped` exacts), rapport couvrant la seule frame orchestrée (vide avant la première, intact après `render_to_target` — devenu la passe immédiate `chaos.offscreen`), et **le checkpoint : le miroir déclaré (ordre -10) + la principale échantillonnant sa couleur — deux frames à l'ordre stable, puis chaque configuration incohérente refusée avec son erreur nommée**) + **material system mature** (le MODÈLE déclare son contrat — layout attendu, shader, entrées —, la DÉDUP : deux materials au même modèle/état = UN `create_pipeline` au journal, chaque état (double_sided, transparent) = sa permutation — suffixe ` blend=alpha` transmis —, UN material sert surface ET cible (les deux permutations aux deux passes — l'ex-duplication offscreen morte), un modèle Custom au shader introuvable échoue à `create_material` (l'eager), le vertex layout d'un mesh désassorti du modèle → draw écarté avec warn, les entrées material sur un modèle sans entrées → refus nommé (create ET setters), `set_material_color` = `update_material_binding` au journal sans AUCUN binding/pipeline recréé + info mise à jour + périmé refusé, `set_material_texture` transactionnel — nouveau binding, l'ancien en retraite vidée au flush, refcounts déplacés (l'ancienne texture redevient destructible, la nouvelle refusée), no-op si identique, cubemap refusé, le handle SURVIT —, la partition opaque-puis-transparent (soumis à l'envers → rendu dans l'ordre, `render_to_target` pareil — la fonction partagée), `material_info` = l'inspection complète reflétant les mises à jour, les stats comptant les permutations lazy, un feedback introduit par `set_material_texture` attrapé au resolve suivant, et **le checkpoint : 3 meshes dont 2 partagent un material, la couleur de l'autre modifiée entre deux frames sans recréation, la passe miroir au MÊME material, le mesh au mauvais layout écarté — partage, distinction, modification, contrats validés**) + **lighting V1** (`LitVertex` stride 32 aux offsets exacts et octets entrelacés, `LitGeometry` : le cube aux 24 normales de face UNITAIRES — elles cessent d'être jetées — et le quad en +Z, le modèle `Lit` résolvant sa permutation `chaos.material.lit` et dessinant sur mesh éclairable, un mesh TexturedVertex sous material `Lit` écarté (le contrat layout), les lumières soumises ATTEIGNENT le backend — ligne `lights` du journal : ambiante exacte, compte, direction NORMALISÉE à la collection, données ponctuelles packées —, une frame sans éclairage n'émet AUCUNE ligne (les tests exacts historiques le verrouillent), la troncature à 16 en ordre de soumission (la 17e n'entre pas) avec réarmement de l'épisode sous la limite, les désactivées filtrées à la collection et les INVALIDES (direction nulle, intensité négative, cône plat outer==inner) écartées AU SUBMIT, `clear_draws` vide les lumières mais l'AMBIANTE persiste (le réglage), UNE ligne lights pour tout le plan (les passes partagent), `render_to_target` passe par la MÊME collection (désactivées filtrées aussi), un plan vide n'envoie rien, le verrou `MAX_LIGHTS` Rust↔WGSL (la source embarquée doit déclarer `array<GpuLight, 16>`), et **le checkpoint : material Lit + mesh éclairable + directionnelle + 2 ponctuelles + spot sur deux frames — le toggle `enabled` observable (le soleil disparaît de la collection), les draws résolus dans les deux**) + **PBR V1** (les défauts = un diélectrique mat éteint et `first_pbr_property` les détecte, les propriétés PBR hors défaut REFUSÉES sur Unlit/Lit en NOMMANT la propriété — acceptées sur Pbr et Custom-avec-inputs (la délégation au shader custom) —, le modèle Pbr au contrat verrouillé (LitVertex, chaos.pbr, tag) et sa permutation `chaos.material.pbr`, le binding aux 7 SLOTS toujours remplis — ligne mock `mr=/normal=/ao=/em=` aux indices de fallbacks déterministes (la blanche partagée, la normale plate) + suffixes de params hors défaut —, cubemap refusé sur CHAQUE slot, CHAQUE slot refcounté (destroy refusé en nommant, destroy_material libère tout, la même texture sur DEUX slots = deux parts symétriques), `set_material_metallic`/`roughness`/`emissive` in-place (update au journal, zéro binding/pipeline créé, info reflétée, refus sur Lit, périmé refusé), le feedback attrapé sur un slot PBR (l'émissif portant la couleur de la cible de sa passe → draw écarté), la position caméra voyageant PAR PASSE (suffixe `cam=` hors défaut, `render_to_target` sans caméra — documenté), la sphère UV (normales radiales unitaires, CCW, UV bornés, résolution clampée sous u16 — jamais d'écrasement silencieux), et **le checkpoint : la grille de combinaisons metallic/roughness + normal map + émissif — 6 bindings distincts, UNE permutation, draws résolus avec la caméra, l'émissif modifié entre deux frames sans recréation**) + **environnement & ciel V1** (l'environnement VALIDÉ — texture 2D refusée en nommant label et kind, handle périmé refusé, intensité non finie ou négative refusée —, le rebind backend UNIQUE (`set_environment index=` au journal ; re-poser le MÊME cubemap = mise à jour intensité/ciel sans rebind ; un autre = rebind ; clear → « none », idempotent), la cubemap ACTIVE indestructible (« clear it first » — OK après clear), le draw CIEL injecté dans les passes Clear seulement — après les opaques, avant les transparents, le tuple sans buffers/binding au journal, compté au rapport ; une passe `Keep` ne le reçoit JAMAIS ; `render_to_target` (toujours Clear) le reçoit —, sky=false coupe le draw mais les uniforms voyagent, SANS environnement zéro delta de journal (les tests exacts historiques le verrouillent), UNE permutation ciel par format (`chaos.sky[.Format]`, suffixe ` depth=less_equal`, cache stable sur deux frames), la ligne `environment intensity= exposure=` hors défaut seulement, l'exposition validée (0/négatif/NaN/inf refusés) et PERSISTANTE (survit à `clear_draws`), `environment_info` reflétant l'état (label, intensité, ciel, mips), les mips `Generate` étendus — cubemap filtrée PAR FACE (moyennes préservées face à face, layout niveau-majeur), `Rgba16Float` moyenné EN FLOTTANT (le HDR > 1 préservé, verrouillé en binary16 exact), formats bornés en nommant —, et **le checkpoint : grille PBR + cubemap HDR mippée + ciel — le ciel dans les DEUX passes, l'exposition et l'intensité réglées sans rebind ni recréation, l'environnement effacé → le ciel disparaît**) + **Shadows V1** (le descripteur VALIDÉ avant le backend — bornes de résolution nommées 16..=8192, volume dégénéré et biais non finis refusés —, `set_shadow` backend UNE fois (re-poser la même résolution avec d'autres biais = zéro appel — le tuning à chaud ; une autre résolution recrée ; clear → « none », idempotent), la passe d'ombre en TÊTE du journal avec ses casters (binding None, résolution, index de lumière), sans directionnelle (aucune, désactivée, ou ponctuelle seule) → AUCUNE passe et rapport None — jamais fatal, la PREMIÈRE directionnelle de la collection projette (une ponctuelle soumise avant → `light=1`), transparents et `without_shadow_cast` jamais casters, le ciel injecté jamais caster (la collecte vit à la branche opaque), permutations d'ombre par (layout, culling) mémoïsées — labels `chaos.shadow.{stride}[.double_sided]`, stables entre frames —, un layout sans position Float32x3@0 écarté et mémoïsé, `render_to_target` sans passe d'ombre et rapport intact, plan vide → ombre sautée aussi, `receive_shadows` hors défaut REFUSÉ sur les modèles non éclairés en nommant la règle (accepté sur Lit/Pbr/Custom-avec-inputs, suffixe ` recv=off` au journal binding), `directional_shadow_info` miroir de l'état, la famille `shadow_maps` des stats (résolution² × 4, retour au niveau de base au clear), et **le checkpoint : la scène complète (caster, non-caster, transparent, receive-off) sur trois frames — deux casters puis un, le volume et les biais retouchés À CHAUD sans set_shadow backend, le soleil coupé → la passe disparaît, l'effacement → stats et rapport au niveau de base**) + **Transparency & Ordering V1** (les CONTRATS déclarés par catégorie — writes_depth/blends/casts_shadows/entrée fragment/suffixe de label, l'autorité unique `MaterialOpacity` —, Masked refusé sur un modèle sans entrées (« no alpha to test »), le cutoff hors défaut refusé hors Masked et borné 0..=1 en nommant, la permutation masked = UN pipeline distinct au label `.masked` et à l'entrée ` entry=fs_masked` (blend REPLACE — jamais ` blend=alpha`), le cache dédupliquant les masked du même modèle, `set_material_alpha_cutoff` IN-PLACE (update au journal, zéro binding/pipeline créé, info reflétée, refus nommés hors Masked et hors bornes, suffixe ` cutoff=` hors défaut), l'ordre à QUATRE TEMPS au journal (opaque → masked → ciel → transparent, soumis à l'envers), le tri des transparents ARRIÈRE → AVANT (par distance² à la caméra de la passe), le tri qui SUIT la caméra (deux frames, la caméra passe derrière → l'ordre s'inverse), les égalités de distance STABLES (l'ordre de soumission conservé), masked PROJETTE sa silhouette (le transparent jamais — le contrat), la VENTILATION par catégorie au rapport (`DrawBreakdown` exact, injected = le ciel, vide sur passe désactivée), et **le checkpoint : les trois catégories sous ombres et ciel sur deux frames — ventilation {1 opaque, 1 masked, 2 transparents, 1 injecté}, 2 casters, l'ordre des verres inversé quand la caméra passe derrière, le cutoff retouché à chaud sans recréation**) + **Instancing V1** (le layout d'instance VERROUILLÉ — stride 128, huit Float32x4 aux locations 4..=11, cadence Instance, `packed_at` généralisant `packed` —, le packing 128 o/instance aux offsets exacts, les runs (material, mesh) FUSIONNÉS à partir de 2 (`inst=N` au journal, la permutation `.instanced` dédupliquée entre materials du même modèle), le motif « un mesh, N draws » devenu UN draw instancié, les TRANSPARENTS jamais instanciés (le tri prime), masked et opaque JAMAIS dans le même batch (permutations `.instanced` vs `.masked.instanced` à l'entrée `fs_masked`), le chemin CLASSIQUE intact (16 meshes distincts = 16 draws en ordre), `render_to_target` batché AVEC le format de sa cible dans le descripteur (le bug attrapé au run GPU, verrouillé au mock), les casters d'ombre MOISSONNÉS puis fusionnés duplicatas multi-passes compris (`ShadowReport.draw_calls`), `draws` = objets logiques / `draw_calls` = soumissions aux deux rapports, et **le checkpoint : 504 objets (500 compatibles + 1 solitaire + 3 transparents) → 5 draw calls, l'ombre 501 casters → 2, AUCUN pipeline de plus à la frame suivante**) + **Visibility & Culling V1** (les bounds LOCAUX calculés à la création — cube exact ±0.5, géométrie vide et positions non finies → `None` avec warn, `mesh_bounds` périmé refusé —, le `Frustum` pur — ortho dedans/dehors/frontière INCLUSIVE, perspective rejetant derrière la caméra, la caméra-échelle du banc mock —, un draw hors champ REJETÉ (plan vide au journal, `culled: 1` au rapport, zéro draw call), la boîte à cheval sur le bord JAMAIS rejetée (le conservatisme est le contrat), un mesh SANS bounds jamais cullé (le défaut sûr), `without_frustum_culling` sautant les DEUX tests (dessiné ET moissonné hors de tout frustum), les transparents cullés AVANT le tri, l'instancing ne fusionnant QUE les visibles (10 objets dont 5 dehors → `inst=5`, `culled: 5`), **l'ANTI-POP** : un caster hors caméra mais dans le volume de lumière ABSENT de la passe et PRÉSENT dans l'ombre — et l'inverse : visible hors volume, dessiné mais jamais moissonné —, chaque vue cullant avec SON frustum (l'overlay à caméra décalée garde SES objets, la principale les siens), `render_to_target` cullé par la VP qu'on lui donne, et **le checkpoint : 1 001 objets dont ~900 hors champ → 100 résolus en UNE soumission (`culled: 901`), l'ombre en résout 101 — celui DERRIÈRE la caméra projette encore —, deux frames aux comptes exacts sans pipeline de plus**) + **Debug Rendering V1** (la TESSELLATION pure aux comptes VERROUILLÉS — ligne 2 sommets, rayon 8, flèche 10, point 6, marqueur-octaèdre 24, axes 6 aux couleurs canoniques RVB (celle du draw ignorée), grille `2×(2n+1)` segments, aabb 24, sphère 144, frustum 24 (les coins d'une ortho retombent EXACTEMENT sur son volume — l'aller-retour), lumières : ponctuelle = cercles À SA PORTÉE + croix, directionnelle = 3 flèches, spot = 2 cercles + génératrices —, les builders portant leurs réglages (couleur, durée, catégorie, overlay, passe — la lumière prend SA couleur), la validation AU SUBMIT refusant en nommant (non-fini, tailles/pas/rayon non positifs, durée négative, VP non inversible, catégorie vide, passe inconnue — jamais stocké), le ROUTAGE par durée (0 = frame, vidée par `clear_draws` ; > 0 = retenue qui SURVIT et expire par `advance_debug_time` — delta invalide ignoré avec warn), les toggles global et par catégorie, le batch INJECTÉ après les transparents (suffixe ` dbg=[v=N p=P]` du journal, permutation `chaos.debug` lignes + blend + LessEqual sans écriture), l'OVERLAY en dernier sur SA permutation (`chaos.debug.overlay`, ` depth=always`), le routage par PASSE cible (chaque passe son debug, la cible sa permutation de FORMAT), le debug coupé = la ligne EXACTE historique (zéro delta), la catégorie filtrée au RENDU (les retenues expirent en coulisses et réapparaissent), la retenue dessinée à travers `clear_draws` puis DISPARUE à l'expiration, `render_to_target` sans debug (la règle de la passe d'ombre), les comptes du rapport (chaque primitive dessinée = un objet `injected`, chaque batch = un `draw_call` — `draw_count()` intact), et **le checkpoint : toutes les formes sous les deux profondeurs + une scène régulière — 12 injectées en 2 batches (3 soumissions), la retenue seule survit à la frame suivante SANS pipeline de plus, sa catégorie coupée la cache pendant qu'elle expire, le journal redevient exactement historique après l'expiration**) + **Diagnostics & GPU Profiling V1** (le snapshot aux défauts HONNÊTES — GPU `Unavailable` avec raison, budget `None` = jamais de dépassement —, le `Display` portant les chiffres clés, le mock DÉCLARANT son temps GPU indisponible (« aucune valeur inventée », verrouillé), le budget CPU validé (non fini/≤ 0 ignorés avec warn) et comptant les dépassements (minuscule → chaque frame dépasse, retiré → le cumul reste), les compteurs EXACTS sur scènes connues — la foule instanciée (504 soumis, 4 classiques + 1 batch de 500, 1 008 triangles, 3 switches pipeline, 3 material), le culling (10 soumis → 5 résolus, 5 cullés, 60 triangles des seuls VISIBLES), le ciel + debug (1 triangle injecté, 4 segments à part, jamais mélangés) —, les événements de surface CUMULÉS par raison (Rendered/indisponible/reconfigurée/aire nulle), les fallbacks VISIBLES (une permutation d'ombre impossible mémoïsée = 1 chemin dégradé, les builtins comptés), `render_to_target` n'y touche pas, le plan VIDE a aussi son snapshot (zéro événement de surface — rien n'est parti au backend), et **le checkpoint : la scène composée (foule + cullés + masked + transparents + ciel + ombre + debug deux modes) sur deux frames — CHAQUE champ exact (208 soumis → 206 résolus, 200 instances en UN batch, 5 cullés, 407 triangles + 4 segments, 6/3 switches, l'ombre 201 → 2 avec 5 rejets), les compteurs STABLES entre les frames, les cumulatifs qui avancent, les ressources = `resource_stats()`**) + **Cross-Platform Robustness V1** (les défauts `DeviceLimits` = le plancher WebGPU (8192 px, 256 Mio, alignement 256, anisotropie ×16), le `Display` du rapport expliquant CHAQUE décision (domaine, statut, détail — fallback et disabled portent leur raison), le rapport mock DÉTERMINISTE (timestamps `Disabled` avec raison — aucune feature optionnelle supposée — et la consultation n'écrit RIEN au journal), les REFUS device nommés (texture 8193 → « exceeds the device texture limit (8192) », render target pareil, la limite EXACTE acceptée), un device ABAISSÉ qui parle en son nom (texture 2048 légale pour l'engine refusée par CE device, buffer 1001 octets refusé par les DEUX chemins — public et meshes —, l'ombre 2048 validée par le descripteur mais refusée par le device en nommant SA borne, l'anisotropie ×16 refusée par un plafond device ×8 distinct du self-check du descripteur), et **le checkpoint : le rapport complet inspecté (chaque domaine expliqué, les raisons non vides), les timestamps coupés n'empêchent RIEN (la frame rend, le GPU est dit indisponible), une configuration impossible échoue proprement et la scène rend encore, le journal n'a jamais rien vu**) + **la SUITE stress & régression** (12 tests noire-boîte dans `src/suite.rs` sur le banc partagé `src/testing.rs` — la scène canonique, le churn, les tempêtes, les longs runs, la performance : le détail en section 1bis) + 11 tests d'intégration : 2 d'**isolation wgpu**, 9 de **validation naga des `.wgsl` intégrés** (compilation + conventions d'inputs — les 7 slots material, les bindings environnement ET les bindings ombre (4–5) du groupe frame — + capacité lights sur les DEUX shaders éclairés + **le verrou miroir FrameUniforms** : PBR et SKY déclarent `inverse_view_projection`/`environment_params`, SKY échantillonne une `texture_cube` + **le verrou miroir de la queue OMBRE** : LIT et PBR déclarent `shadow_view_projection`/`shadow_params`/`texture_depth_2d`/`sampler_comparison` + **le verrou du shader d'ombre** : `chaos.shadow` expose UN point d'entrée vertex et ne binde que (0,0) et (1,0) — le layout réduit de sa passe — + **le verrou des entrées MASKED** : TEXTURED, LIT et PBR exposent `fs_masked` à côté de `fs_main` (un material masked échouerait à la création du pipeline sur GPU seulement) + **le verrou des entrées INSTANCIÉES** : les CINQ shaders à géométrie exposent `vs_instanced` (même garde) + **le verrou du shader DEBUG** : `chaos.debug` expose EXACTEMENT `vs_main`/`fs_main`, ne binde que la vue-projection (0,0) et ses entrées vertex reflètent `DebugVertex::layout()` (position vec3 @0, couleur vec4 @1) ; la seule garde du miroir layout↔WGSL, aucun test GPU n'existe) |
| `chaos_engine` | 135 | Cycle de vie complet (init/shutdown ordonnés, exits, gating, échecs d'init, update → render) + **contrôleur de caméra debug** (avance selon forward, purge au focus perdu, rotation au drag droit seulement, pas de saut au premier mouvement, pitch clampé, vitesse bornée à la molette) + **couture assets → renderer** (mapping texture/géométrie exacts, garde u16 des gros meshes — lit comprise —, appariement UV préservé, `lit_geometry` : normales du fichier appariées, SYNTHÈSE plate exacte quand absentes — +Z sur le quad du sol réel —, sommet dégénéré retombant sur +Y sans NaN) + **intégration ECS** (la ressource `Time` alimentée par le tick, un événement pompé = un message pour exactement un update, un système enregistré via `schedule_mut` tourne à chaque frame, un système en échec arrête le moteur proprement, la propagation des transforms garantie à chaque update — parent/enfant composés dans `stages::POST_UPDATE`, le shutdown moteur nettoyant les scènes — monde et manager vides) + **couture scènes ↔ pipeline** (save→declare→load rebouclant sur fichiers réels, référence d'asset inconnue en erreur explicite, fichier corrompu échouant proprement, **scène réelle de bout en bout** — save→declare→load→activate→update avec globaux frais — et **confinement d'erreur** : un chargement corrompu laisse monde et manager vides, le moteur continue) + **cycle de vie mature** (pause gelant updates/temps/messages mais pas le rendu, reprise à la frontière de frame sans saut, requête périmée purgée au démarrage, requête hors état écartée, double start refusé, shutdown répété idempotent et moteur silencieux ensuite) + **modèle de frame verrouillé** (trace exacte UPDATE → LATE_UPDATE → POST_UPDATE → subsystems sur deux frames, subsystems en ordre d'enregistrement dans chaque hook, **ordre strictement identique entre deux runs**, événements visibles des systèmes de la même frame) + **système de temps** (cadence fixe bornée exacte — pas minuscule saturant le cap, pas énorme = zéro pas —, FIXED_UPDATE avant les stages variables, scale 0 ≠ pause — systèmes tournant à delta nul —, aucun pas fixe fantôme en pause, échelle invalide refusée) + **ordre des subsystems** (dépendances par nom → ordre topologique exact avec shutdown inverse, égalités départagées par l'enregistrement, cycle/dépendance absente/noms dupliqués refusés proprement — aucun init, moteur jamais Running —, échec d'init nettoyé en inverse TRIÉ, tous les hooks suivent l'ordre trié) + **frontières des services** (communication inter-subsystems par le World seul — producteur/consommateur par messages sans se connaître —, enregistrement post-init appliqué à la frame suivante, services utilisables dans init/update/shutdown, **verrou CI anti-globals** : aucun static mut/once_cell/thread_local dans tout le workspace) + **système de configuration** (défauts valides, surcharge d'application valide, chaque règle de validation refusée avec son erreur précise — nom d'app vide, fenêtre 0, couleur non finie, fps 0, pas fixe nul, filtre de logs vide, headless réservé, frame_limit 0, désactivations dupliquées/vides —, **une configuration invalide échoue AVANT toute initialisation partielle** — journal vide, erreur exploitable —, subsystems désactivés par configuration jamais initialisés ni tickés, désactivation d'un subsystem inconnu refusée proprement) + **exécution headless** (`run()` headless exécutant EXACTEMENT N ticks puis s'arrêtant proprement — init/update×N/shutdown, jamais de `render` —, subsystems graphiques retirés en headless et gardés en fenêtré, dépendre d'un graphique retiré refusé en nommant les deux, run non borné arrêté par le `request_exit` d'un subsystem — la sémantique serveur —, requête de pause écartée en headless — le frame_limit aboutit quand même —, run cadencé terminant, échec d'init remonté par `run()`, et **l'application headless complète de bout en bout** par l'API publique seule : scène réelle chargée via l'Asset Pipeline, système variable + pas fixe + hiérarchie propagée, N ticks exacts, arrêt propre — `tests/headless.rs`) + **modèle d'erreurs et de défaillances** (une fatale d'EXÉCUTION ressort de `run()` en `Err` précis — plus jamais un exit 0 sur défaillance —, la frame de l'échec ECS est ABANDONNÉE — aucun update sur un monde en état inconnu —, l'escalade `report_fatal` d'un subsystem arrête proprement avec SON diagnostic, la première défaillance gagne le diagnostic — les suivantes loguées comme conséquences —, une erreur récupérable gérée localement ne stoppe jamais le moteur — le run va au bout, `Ok` —, une demande d'arrêt normale n'est pas une erreur) + **diagnostics & profiling CPU** (le snapshot `last_frame()` cohérent — les 3 stages nommés dans l'ordre, les subsystems nommés dans l'ordre trié, `fixed_steps` exacts, `total ≥ update ≥ fixed + Σstages + Σsubsystems`, les spans dormeurs ≥ leur sommeil —, les dépassements comptent le TRAVAIL contre le budget — jamais l'attente de cadence, budget `None` = jamais de dépassement —, la fréquence fixe rapportée, les spans de render enregistrés, le snapshot est TOUJOURS une frame complète — jamais la frame en cours —, la pause ne pollue pas le profil, et le mécanisme du double-buffer : clôture/échange, no-op sans frame ouverte, slots réécrits en place sans croissance) + **metrics de santé** (LE checkpoint : une application lit un snapshot cohérent PENDANT l'exécution — entités/scènes actives/assets chargés exacts, draws 0 en headless, octets suivis > 0, subsystems nommés, daté de la frame close —, les temps de frame viennent de la fenêtre glissante — min ≤ avg ≤ max, min ≥ le sommeil, fps cohérent avec avg —, erreurs et avertissements comptés aux chemins moteur, les statuts reflètent les décisions du démarrage — Active/Disabled/SkippedHeadless exacts —, les jauges à zéro échantillonnées honnêtement, et le mécanisme : fenêtre exacte sur durées connues, rollover aux 120 dernières, compteurs cumulatifs, fenêtre vide cohérente) + **interruptions** (perte de focus → pause auto SI la politique `pause_on_focus_loss` l'active — rien sinon —, le retour de focus ne reprend QUE la pause auto — une pause app survit aux allers-retours de focus et à la suspension —, `Suspended` pause et coupe les hooks render / `Resumed` rétablit tout, **LE checkpoint : 300 ms d'interruption réelle → `delta` quasi nul (horloge RESYNCHRONISÉE à la reprise — plus aucun saut visible, plus aucune rafale de pas fixes), le temps de jeu n'a pas avancé, `real_elapsed` ≥ 300 ms — la vérité murale conservée**, aucune touche fantôme ne traverse une interruption — la touche pompée avant est balayée, le premier update après reprise voit zéro message clavier —, et la purge du contrôleur debug sur suspension comme sur perte de focus) + **garanties d'arrêt** (LE checkpoint : l'invariant post-arrêt — World vide, zéro message, scènes vides, zéro asset chargé, zéro octet en cache, renderer absent — vérifié sur la matrice des scénarios : frame_limit, demande d'un subsystem, fermeture système, **défaillance fatale** — l'arrêt ordonné n'est pas réservé au chemin heureux —, échec partiel d'init — seuls les initialisés arrêtés, les ressources du réussi fermées quand même —, travaux en attente annulés — messages drainés, pause pendante purgée —, et un moteur arrêté qui RESTE arrêté : double shutdown sans double libération, `start()` refusé, hooks muets) + **frontière d'application** (les résultats lisibles APRÈS le run — `Engine::diagnostics()`/`metrics()` exacts post-arrêt —, un second `run()` refusé avec erreur précise — un cycle de vie par Engine, fini le no-op menteur —, et le **verrou CI de façade** : `apps/` ne dépend et n'importe JAMAIS une crate interne du moteur — manifestes et sources scannés, plateforme permise) + **tests de stress** (`tests/stress.rs`, par l'API publique seule : 10 000 ticks headless stables aux compteurs exacts, 100 subsystems en chaîne de dépendances enregistrés à l'envers — tri, init et shutdown exacts —, 10 000 entités mutées et propagées à chaque frame plus une chaîne hiérarchique de 100, un déluge de 10 000 messages/frame jamais cumulé sur 50 frames, 100 cycles create/load/activate/unload de scènes sans résidu ; + 1 000 cycles pause/reprise cohérents sans saut de temps) |

Les tests unitaires ne touchent jamais le GPU (la CI n'en a pas) : la validation
GPU est locale, via les runs sandbox ci-dessous.

Cibler une crate et voir le nom de chaque test :

```sh
cargo test -p chaos_engine
```

## 1bis. La suite stress & régression (`chaos_renderer::suite`)

```sh
cargo test -p chaos_renderer suite
```

La suite DURABLE qui protège le renderer mature V1 — NOIRE-BOÎTE (l'API
publique, les diagnostics, les stats : le niveau contrat), dans son
propre module (`src/suite.rs`), sur le banc d'essai partagé
(`src/testing.rs` — le `MockBackend` à journal, extrait de
`renderer.rs`, à l'issue COMMUTABLE : pertes de surface et erreurs
backend se scénarisent en cours de run). Quatre familles, douze tests :

- **LA SCÈNE CANONIQUE** (le cœur) : environnement HDR + ciel + IBL,
  4 lumières + ambiante, ombres (volume explicite), sol lit texturé
  (sampler trilinéaire anisotrope ×8), 2 sphères PBR, grille masked,
  2 verres (couleur modifiée À CHAQUE frame — l'update in-place), foule
  INSTANCIÉE de 100 + 50 HORS CHAMP (cullés), passe MIROIR déclarée
  (cible 128², 10 cubes redessinés) + écran qui l'échantillonne, debug
  (grille Scene + axes Overlay + marqueur retenu qui EXPIRE). Le
  checkpoint : 10 frames animées — 167 soumis → 122 résolus (50
  cullés, 5 injectés, 110 instances en 2 batches, 57 segments), l'ombre
  115 → 6, les 12 domaines assertés, les diagnostics IDENTIQUES frame à
  frame, les ressources CONSTANTES caches chauds.
- **Ressources** : le churn INTENSIF (300 cycles create/partage/swap de
  texture/destroy + render de flush → la mémoire REVIENT à la baseline,
  seuls les pipelines — permanents par contrat — restent) ; les ordres
  de destruction INVALIDES en rafale (partagée, fallback protégé,
  double destroy — refusés en nommant, la scène rend encore) ; la
  ROTATION de render target (50 resizes + rebind : la couleur se
  détache avant la rotation, zéro fuite après destruction).
- **Robustesse** : la TEMPÊTE de resize (200 alternances, 0×0 compris,
  un render entre chaque — les comptes inchangés) ; les pertes et
  récupérations de SURFACE commutées en cours de run (compteurs cumulés
  exacts, la passe surface `SurfaceSkipped`, la passe cible s'exécute
  quand même) + l'ERREUR backend injectée (le `Err` remonte de
  `render_frame`, la frame suivante est saine) ; le LONG RUN (1 000
  frames canoniques : diagnostics identiques à chaque centième, mémoire
  CONSTANTE — la garde de dérive —, retraite toujours drainée) ; les
  erreurs de l'UTILISATEUR en boucle (texture trop grande, lumière
  invalide, mesh périmé, passe inconnue, debug non fini — 50 itérations
  sans troubler le rendu) ; le backend RÉDUIT (limites abaissées,
  timestamps absents : la scène-lite rend, les refus parlent device).
- **Performance** : beaucoup de TOUT (1 128 objets — deux foules de
  500 fusionnées en 2 batches, 100 materials, 30 textures + fallbacks,
  18 lumières soumises → 16 gardées, 4 passes) : les soumissions
  BORNÉES par l'instancing, AUCUN pipeline ni octet de plus à la frame
  suivante ; les budgets CPU (généreux → jamais de dépassement,
  minuscule → chaque frame).

Le partage GPU réel / sans GPU : la suite et les unitaires couvrent
l'orchestration et les contrats ; les shaders sont verrouillés par naga
(9 tests) ; le backend, le résultat visuel et la stabilité passent par
les runs Metal ci-dessous (section 2quater — le long run).

## 2. Test end-to-end automatisé (sans interaction)

```sh
CHAOS_FRAME_LIMIT=180 cargo run -p sandbox
```

La fenêtre s'ouvre, le moteur tourne 180 frames (~3 s à 60 fps) puis s'arrête seul. Séquence attendue dans les logs :

```
INFO  Chaos Sandbox starting (Chaos Engine <version>)
INFO  window ready: <w>x<h> (scale factor <n>)
INFO  graphics adapter selected: wgpu (<GPU> / <Backend>)
INFO  renderer ready: wgpu (<GPU> / <Backend>)
INFO  gpu capabilities: all domains active (<Backend> on <GPU>)
INFO  scene 'scenes/demo' loaded (4 entities)
INFO  engine running (2 subsystem(s))
INFO  frame limit reached (180), requesting exit
INFO  engine shutting down
INFO  renderer released
INFO  diagnostics: 180 frame(s), <m> over budget
INFO  engine stopped
INFO  Chaos Sandbox stopped cleanly
```

Les 2 subsystems : `geometry_demo` (contenu) + `render_subsystem` (pilote,
enregistré automatiquement en dernier).

Le code de sortie doit être `0` (`echo $?` juste après).

**Lancer depuis la racine du workspace** (le sol vient de fichiers réels :
`assets/textures/checker.ppm` et `assets/models/floor.glb`, chargés par
l'Asset Pipeline — declare → import → couture → renderer). **La scène
`scenes/demo` (sol + cube central + satellites) est CHARGÉE depuis le
fichier committé `assets/scenes/demo.cscn` — aucune entité n'est
construite dans le code de la démo** ; les références d'assets sont
résolues au chargement. La fenêtre doit
afficher la **scène pilotée par les materials** — 16 draws
par frame (triplets mesh + material + transform) pour **5 meshes** et **6
materials** : un **sol damier violet** (quad texturé 1×1 étiré en 20×20 —
la scène est organisée en PAVILLONS espacés, le plan est dans
`geometry_demo/mod.rs` —, posé
à y=-1 — damier 64×64 **neutre blanc/gris** (carreaux de 32 px) répété ×4,
MIPPÉ (`TextureMips::Generate`, 7 niveaux) et lu par le sampler
trilinéaire anisotrope ×8 `demo.floor_sampler` — net de près, fondu
propre au loin, sans shimmer —, teinté par le `base_color` violet du material
`demo.floor`), un **cube central damier ambre** en rotation lente sur deux
axes (`TexturedGeometry::cube` : UV 0..1 sur chaque face — le MÊME damier
que le sol, teinté ambre par le material `demo.cube` : deux materials, une
texture partagée ; **le cube est une entité ECS** : composants `Transform` +
`Spin`, animé par le système `demo.spin` dans `stages::UPDATE` ; **deux
satellites attachés** (`hierarchy::attach`, échelle 0.3) suivent rigidement
son spin par la seule propagation `stages::POST_UPDATE` — zéro mise à jour
manuelle d'enfant, les DrawCommands lisent les `GlobalTransform`), une **ronde de 8 cubes 6 couleurs** (mesh coloré partagé,
material `demo.solid` sans texture → fallbacks builtin `chaos.white` +
`chaos.default_sampler`) de tailles (0.3 → 0.72) et vitesses d'orbite/spin
toutes différentes, et **trois triangles dégradés** flottants. Les vitesses
d'orbite différentes font que les cubes **se croisent en permanence** :
l'occlusion doit rester correcte à chaque croisement. S'y ajoutent
**l'écran de surveillance** (rendu hors écran, sous-phases 4 et 5 de la
consolidation) : la frame est ORCHESTRÉE en deux passes déclarées — la
passe `demo.mirror` (ordre -10, `add_pass`/`queue_draw_to`) rend la ronde
des cubes vue du dessus (caméra fixe) dans la cible `demo.mirror`
(256×256) **avec le MÊME material `demo.solid` que la scène** (la
permutation de pipeline du format de la cible se résout seule — plus
aucune duplication offscreen), puis la passe `chaos.main` dessine la
scène, dont un **quad flottant** qui affiche cette texture (material
`demo.screen` : couleur de la cible + sampler ClampToEdge) — et **le
panneau de verre `demo.glass`** (sous-phase 6) : un quad TRANSPARENT
bleuté dont l'alpha PULSE chaque frame via `set_material_color` (la mise
à jour in-place : 16 octets écrits, zéro recréation), rendu APRÈS les
opaques de sa passe — et **L'ÉCLAIRAGE** (sous-phase 7) : le sol et les
cubes de la scène sont des materials `Lit` (le damier et les teintes
violet/ambre RÉAGISSENT à la lumière — meshes à normales, celles du sol
SYNTHÉTISÉES car `floor.glb` n'en porte pas), sous une ambiante douce
(0.08), une DIRECTIONNELLE chaude (togglée par **K** — le sol s'assombrit
nettement), **trois PONCTUELLES colorées (rouge/verte/bleue) qui
orbitent** — leurs flaques de couleur balaient le sol, chacune suivie de
son petit cube marqueur — et un SPOT cyan plongeant sur le cube central.
S'y ajoutent **le SHOWCASE PBR** (sous-phase 8) : la grille 4×4 de
sphères `Pbr` gris clair flottant derrière la scène (metallic croissant
vers la droite — le reflet se teinte —, rugosité croissante vers le bas —
le highlight s'étale), le cube normal-mappé (map procédurale
`demo.bumps`) qui accroche les ponctuelles orbitantes sur ses bosses, et
la sphère émissive orange PULSANTE (`set_material_emissive` in-place
chaque frame) — et **L'ENVIRONNEMENT** (sous-phase 9) : la cubemap HDR
procédurale `demo.sky` (Rgba16Float 64×64, mips générées, gradient +
disque solaire aligné sur la directionnelle) remplace le fond uni par le
CIEL (passes Clear — le miroir le montre aussi) et éclaire la scène en
IBL (l'ambiante plate descend à 0.02) : la colonne métallique de la
grille REFLÈTE le ciel — net en haut (rugosité 0.1), flou en bas ; **E**
bascule l'environnement (retour fond uni + ambiante plate seule), **V/B**
règlent l'exposition (ciel et PBR ensemble) — **LES OMBRES**
(sous-phase 10) : le soleil PROJETTE toute la scène sur le sol damier
(shadow map 2048 sur le volume explicite couvrant sol, ronde et grille —
`demo_shadow_settings`) : les cubes de la ronde, le cube central et ses
satellites, la grille de sphères posent des ombres nettes qui SUIVENT le
spin et les orbites ; la sphère émissive n'en pose AUCUNE
(`without_shadow_cast` — la preuve par l'absence) ; **N** bascule les
ombres (`shadows cleared`/`set` — la map est libérée puis recréée), **K**
coupe le soleil et ses ombres avec lui ; les ombres sont STABLES sous
tous les mouvements de caméra (le volume ne la suit pas) — et
**L'OPACITÉ** (sous-phase 11) : la GRILLE masked du coin opacité
(`demo.grille` — texture procédurale à pastilles TRANSPARENTES sur un
quad `Lit` double-sided, bord droit du sol) laisse voir la scène par ses trous NETS
(`fs_masked` élimine sous le cutoff, la profondeur s'écrit comme un
opaque) tandis que son ombre au sol reste la silhouette PLEINE du quad
(l'artefact V1 des casters non alpha-testés, assumé et documenté), et le
TRIO de verres posés au sol à côté (le panneau bleu pulsant + un rouge
+ un vert, `demo.glass.{0,1}`) se mélange dans le BON ordre sous
n'importe quel angle de survol — le tri arrière → avant suit la caméra
frame après frame — et **L'INSTANCING** (sous-phase 12) : l'ESSAIM
(`demo.swarm` — 1 200 mini-cubes `Lit` dorés en double hélice animée
au-dessus du centre, transforms recalculés CHAQUE frame, soumis UN PAR
UN) fusionne de lui-même en UN draw instancié par passe (principale ET
ombre), et la RONDE historique (un mesh, un material) fusionne pareil —
pas une ligne du consommateur n'a changé — et **LA VISIBILITÉ**
(sous-phase 13) : chaque passe CULL avec SA caméra (le miroir avec sa
vue fixe, la principale avec la caméra pilotable) — voler hors de la
scène fait chuter les draws résolus, rien ne disparaît à tort aux bords
de l'écran (le test conservatif à l'œil), et les ombres des objets hors
champ RESTENT au sol : la moisson d'ombre teste le volume de LUMIÈRE,
jamais l'écran — et **LE DEBUG RENDERING** (sous-phase 14) : la GRILLE
du sol (testée par la profondeur — la scène l'occlut) et les AXES du
monde (OVERLAY — visibles À TRAVERS la scène : les deux modes côte à
côte) au spawn, **X** ajoute les bounds monde de la ronde (la matière du
frustum culling rendue visible — les boîtes jaunes suivent les cubes),
**F** les frustums de la caméra du miroir (cyan) et du volume d'ombre
(ambre), **J** les lumières dessinées comme DONNÉES (sphères de portée
des ponctuelles, flèches du soleil, cône du spot — leurs couleurs),
**T** pose un marqueur magenta RETENU 3 secondes à la position caméra
(il survit aux frames et disparaît SEUL — l'expiration à l'œil), **G**
coupe tout le debug d'un coup. Le rig SOUMET tout chaque frame — les
touches basculent les CATÉGORIES côté renderer, le mécanisme prouvé en
réel.
1 249 draws soumis par frame (8 miroir + 1 241 principale — dont les
1 200 de l'essaim) ; environnement
actif, le renderer INJECTE un draw ciel par passe Clear et les
primitives de debug DESSINÉES (comptées `injected`, jamais dans
`draw_count()`) ; chaque
`PassReport` VENTILE ses OBJETS par catégorie, dit ses SOUMISSIONS
réelles (`draw_calls`) et ses REJETS (`culled`) — la touche **O** log LE
SNAPSHOT des diagnostics (sous-phase 15), mesuré sur M4 Pro/Metal :
`1249 submitted -> 1228 resolved (25 culled, 4 injected) | 29 classic +
5 instanced (1197 instances) | 27449 triangles, 45 debug segments`,
`15 pipelines, 32 materials` de switches, la principale ~1 219 draws →
**34 draw calls** (25 culled au spawn — la frange de l'essaim animé
déborde du champ, le compte fluctue), le miroir 9 → 2, l'ombre 1 245
casters → **27 draw calls** (l'union multi-passes fusionne les
duplicatas — la ronde du miroir rejoint celle de la scène), les coûts
`cpu: resolve 0.23 ms + backend 1.00 ms` et **`gpu: 1.95 ms` MESURÉ par
timestamp queries** (jamais inventé : sans la feature, la ligne dit
`gpu: unavailable (raison)`), les ressources (~17,6 Mo suivis), la
surface et les fallbacks cumulés.
AUCUN pipeline n'est créé par la démo : les 31 materials sont
DESCRIPTIFS et les **27 pipelines** se résolvent seuls — 10 permutations
`chaos.material.*` (dont UNE SEULE `chaos.material.pbr` pour les 18
objets PBR, et `chaos.material.lit.double_sided.masked` pour la grille)
+ les 2 permutations du ciel (`chaos.sky.Rgba8UnormSrgb`
pour le miroir, `chaos.sky` pour la surface) + les 6 permutations
d'ombre (`chaos.shadow.{20|24|32}[.double_sided]` — trois vertex
layouts × deux états de culling) + les **7 permutations instanciées**
(`chaos.material.{vertex_color[.Rgba8UnormSrgb|.double_sided]|lit}.instanced`
et `chaos.shadow.{24[.double_sided]|32}.instanced` — lazy, à la
première frame qui les fusionne) + les **2 permutations debug**
(`chaos.debug` — lignes, blend alpha, LessEqual sans écriture — et
`chaos.debug.overlay` — profondeur Always : par-dessus tout). La démo
soumet en ordre de scène ; la **RenderQueue** regroupe par (material, mesh) avant le
backend — visuellement invisible (géométrie opaque + depth buffer), c'est le
point. **Au resize, les proportions sont conservées** (le sol reste carré,
les cubes ne s'étirent pas) : c'est la caméra qui gère l'aspect ratio, plus
l'étirement NDC. Les logs `debug` montrent les 27 pipelines (les eager à
l'init, les permutations de cible, du ciel, d'ombre, d'instancing et de
debug à la première frame), `shadow map created (2048x2048)`,
21 buffers, 11 meshes, les 7 textures (`demo.checker`, `chaos.white`,
`chaos.normal_flat`, la couleur de `demo.mirror`, `demo.sky` — 262 128
octets, la chaîne de mips complète —, `demo.bumps`, `demo.grille`), la
passe déclarée
(`render pass 'demo.mirror' declared (order -10, …)`), les quatre
samplers, les 31 `material … created`, `instance buffer grown to
156416 bytes` (≈ 1 222 instances × 128 octets), et `object uniform
slots grown to 34` — les slots suivent les DRAW CALLS de la plus grosse
passe (batches debug compris : +2, écrits à l'identité), plus les
objets : 1 200 cubes d'essaim n'en consomment qu'UN (le
pool est PARTAGÉ entre les passes : un submit par passe le permet).
Le log `renderer released` doit apparaître
au shutdown, avant `engine stopped`.

### Navigation debug dans la scène

La caméra se pilote au clavier/souris (contrôleur `chaos_engine::debug`) :

| Contrôle | Action |
|---|---|
| **Clic droit maintenu + souris** | Regarder (yaw/pitch, pitch clampé ±89°) |
| **Z/Q/S/D** (touches physiques WASD) | Avancer / gauche / reculer / droite |
| **Espace / Shift gauche** | Monter / descendre |
| **Molette** | Vitesse de déplacement (0,1 → 100 m/s) |
| **P** | Pause/reprise moteur (simulation gelée, fenêtre vivante) |
| **K** | Active/désactive la lumière directionnelle (le « soleil ») — les ombres disparaissent avec lui |
| **N** | Active/désactive les ombres directionnelles (la shadow map est libérée puis recréée) |
| **E** | Active/désactive l'environnement (ciel + IBL ↔ fond uni + ambiante plate) |
| **V / B** | Exposition ÷/× 1.25 (bornée 0.25–4) — ciel et PBR ensemble |
| **L** | Slow-motion (échelle de temps 1.0 ↔ 0.25) |
| **G** | Active/désactive TOUT le debug rendering (grille, axes, et les catégories réveillées) |
| **X** | Active/désactive les bounds monde de la ronde (catégorie `demo.bounds`) |
| **F** | Active/désactive les frustums — caméra du miroir + volume d'ombre (catégorie `demo.frustums`) |
| **J** | Active/désactive le dessin des lumières (catégorie `demo.lights`) |
| **T** | Pose un marqueur debug RETENU 3 s à la position caméra — il expire seul |
| **O** | Rapport de la dernière frame complète dans les logs (profil CPU : phases, stages, subsystems, budget) + LE SNAPSHOT des diagnostics du renderer : soumis → résolus, culled/injectés, classiques/instanciés/instances, triangles, switches, détail par passe et ombre, coûts CPU mesurés, **temps GPU réel** (mesuré sur Metal, `unavailable (raison)` sinon), ressources, surface, fallbacks, budget |
| **H** | Snapshot santé dans les logs (FPS, temps de frame, entités, scènes, assets, draws, erreurs, mémoire suivie, états des subsystems) |

Perte de focus (alt-tab) → **pause automatique** (le sandbox active `pause_on_focus_loss`), touches et drag purgés, aucune touche fantôme ; le retour de focus reprend la simulation — sauf si la pause venait de **P** : une pause manuelle survit aux allers-retours de focus. La suspension OS purge et coupe le rendu de la même façon.

## 2bis. Test headless automatisé (sans fenêtre ni GPU)

```sh
CHAOS_HEADLESS=1 CHAOS_FRAME_LIMIT=240 cargo run -p sandbox
```

Aucune fenêtre ne s'ouvre, aucun GPU n'est touché : le moteur exécute 240 ticks
de la boucle logique complète (~4 s à 60 fps) puis s'arrête seul — **tournable
en CI sans GPU**. Séquence attendue :

```
INFO  Chaos Sandbox starting headless (Chaos Engine <version>)
INFO  subsystem 'geometry_demo' skipped in headless mode (requires graphics)
INFO  engine running (0 subsystem(s))
INFO  frame limit reached (240), requesting exit
INFO  engine shutting down
INFO  diagnostics: 240 frame(s), <m> over budget
INFO  engine stopped
INFO  Chaos Sandbox stopped cleanly
```

Aucun log `window ready` / `graphics adapter` / `renderer ready` ne doit
apparaître. Le code de sortie doit être `0`. La démo (`geometry_demo`) déclare
`requires_graphics()` : elle est RETIRÉE au démarrage — le run headless du
sandbox prouve le mode d'exécution ; l'application headless complète (scène,
assets, systèmes variable et fixe) est prouvée par le test d'intégration
`chaos_engine/tests/headless.rs`.

## 2ter. Validation Windows — checkpoint EXTERNE (déclaré)

**Windows (DX12 ou Vulkan via wgpu) est une plateforme de première
classe du renderer, validée par un checkpoint EXTERNE** : aucun code
spécifique n'existe (wgpu choisit le backend, l'isolation est
verrouillée en CI), et cette checklist n'est PAS exécutée localement —
macOS/Metal est la plateforme de validation continue (chaque sous-phase
y passe ses runs réels). Le premier run Windows est AUTO-DIAGNOSTIQUANT :
le rapport de capacités loggé à l'init dit le backend choisi, les
limites accordées et chaque décision (fallbacks compris) — toute
divergence est observable immédiatement, jamais implicite.

La checklist du checkpoint externe, sur une machine Windows :

1. `cargo run -p sandbox` — la fenêtre s'ouvre, `graphics adapter
   selected: wgpu (<GPU> / Dx12|Vulkan)` et la ligne `gpu capabilities:`
   dans les logs (les écarts éventuels y sont nommés) ;
2. `CHAOS_FRAME_LIMIT=180 cargo run -p sandbox` — code de sortie 0,
   séquence de logs de la section 2 ;
3. `CHAOS_HEADLESS=1 CHAOS_FRAME_LIMIT=240 cargo run -p sandbox` —
   code 0, aucun log graphique ;
4. la scène complète à l'œil (ombres, ciel, PBR, verres, grille masked,
   essaim, miroir) et **O** — le snapshot des diagnostics avec `gpu:`
   MESURÉ (timestamps DX12/Vulkan) ou `unavailable (raison)` ;
5. les toggles K/N/E/V/B/G/X/F/J/T et le resize/minimisation — aucun
   crash, les logs propres ;
6. `cargo test --workspace` — verts (aucun test ne touche le GPU).

## 2quater. Le LONG RUN GPU — la stabilité prouvée en réel

```sh
CHAOS_FRAME_LIMIT=1800 CHAOS_DIAG_FRAME=1700 cargo run -p sandbox
```

Trente secondes de rendu réel, et LE snapshot des diagnostics loggé à
la frame 1 700 — comparable à la photo de la frame 150 documentée en
section 2 : les MÊMES comptes (34/2/27 draw calls, 27 pipelines), la
MÊME mémoire à l'octet près (~17 640 716, 0 retraite — aucune dérive),
`gpu:` toujours mesuré, ~1 697 présentées. Le code de sortie doit être
`0`. La version headless longue :

```sh
CHAOS_HEADLESS=1 CHAOS_FRAME_LIMIT=2400 cargo run -p sandbox
```

La validation VISUELLE reste humaine (assumé, documenté) : la scène de
la section 2 et la checklist interactive de la section 3 — les golden
images automatisées viendront avec l'infra CI GPU.

## 3. Test interactif du cycle de vie

```sh
cargo run -p sandbox
```

La fenêtre reste ouverte. Redimensionner, déplacer, changer le focus, puis fermer avec le bouton natif : les logs doivent montrer `close requested by the system` suivi de la séquence d'arrêt propre.

## 4. Trace des événements en temps réel

```sh
RUST_LOG=trace cargo run -p sandbox
```

Chaque événement traduit s'affiche : `CursorMoved` (souris), `MouseButton` (clics), `Keyboard { key, state, repeat }` (clavier), `MouseWheel` (molette), `Resized`, `Moved`, `Focused`. C'est la vérification vivante de la frontière de traduction winit → `chaos_core`.

Variante ciblée, moins bavarde :

```sh
RUST_LOG=info,chaos_engine=trace cargo run -p sandbox
```

## 5. Portes de qualité (identiques à la CI)

```sh
cargo check --workspace --all-targets
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Si ces trois commandes et les tests passent en local, la CI de la PR sera verte.

## Leviers de test du moteur

| Levier | Où | Effet |
|---|---|---|
| `CHAOS_FRAME_LIMIT=<n>` | env, lu par `sandbox` | Renseigne `EngineConfig` → `runtime.frame_limit` : arrêt propre après n frames |
| `CHAOS_HEADLESS=1` | env, lu par `sandbox` | Renseigne `runtime.headless` : boucle logique complète sans fenêtre ni GPU |
| `CHAOS_DIAG_FRAME=<n>` | env, lu par la démo | Log LE snapshot des diagnostics du renderer à la frame n — la procédure des longs runs et des comparaisons entre machines |
| `runtime.frame_limit` | code (`EngineConfig`) | Même effet, pour tout hôte du moteur |
| `runtime.headless` | code (`EngineConfig`) | Le mode headless réel : subsystems graphiques (`requires_graphics`) retirés, pas de phase render, pause indisponible |
| `runtime.disabled_subsystems` | code (`EngineConfig`) | Désactive des subsystems par leur nom : jamais initialisés, jamais tickés (nom inconnu = démarrage refusé) |
| `time.target_fps` | code (`EngineConfig`) | `None` = boucle libre (utile en test pour éviter le pacing), `Some(n)` = cadence via l'attente native de l'OS |
| `render.vsync` | code (`EngineConfig`) | `false` par défaut (présentation non bloquante — évite le lag d'interactions macOS), `true` = synchronisation écran |
| `logs.filter` | code (`EngineConfig`) | Filtre de logs par défaut appliqué par l'application (`Some("info")` par défaut) — `RUST_LOG` garde la priorité |
| `RUST_LOG` | env (`env_logger` dans sandbox) | Niveau de logs : `error`/`warn`/`info`/`debug`/`trace`, filtrable par module |
