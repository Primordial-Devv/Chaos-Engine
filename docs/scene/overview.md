# Scene System — architecture

Référence des choix de la phase 5. Le principe directeur : **une scène décrit et organise une portion de monde — elle ne possède jamais les données vivantes**. L'ECS fait vivre les entités, l'Asset Pipeline fournit les ressources, le renderer affiche ; le Scene System organise et conserve leur état. Jamais un second ECS.

**Statut : sous-système STABLE** (phase 5 validée). Le moteur sait organiser ses entités en scènes, construire des hiérarchies, sauvegarder leur état, recharger un monde, instancier des structures réutilisables et gérer plusieurs scènes — **la couche de structure et de persistance au-dessus de l'ECS, alimentée par l'Asset Pipeline, consommée par les systèmes futurs. La fondation de Chaos Editor.**

## Les règles de la phase

- **La frontière de possession, tranchée** : la scène possède son identité (`SceneId`), ses métadonnées et son état de cycle de vie ; le `World` (unique, tenu par le moteur) possède les entités, composants, ressources et messages. Détruire une scène ne peut pas corrompre le monde — verrouillé par test.
- `chaos_scene → chaos_ecs` est l'unique dépendance latérale autorisée du moteur (documentée dans `architecture/overview.md`) — jamais l'inverse ; le renderer et l'Asset Pipeline ignorent le Scene System.
- Les scènes référencent les ressources par **identités stables** (`AssetId`), jamais par chemins bruts.
- Zéro logique gameplay, zéro logique renderer ; `sandbox` ne consommera que l'API haut niveau.

## Feuille de route Phase 5

| Sous-phase | Destination | Statut |
|---|---|---|
| 1. Scene Identity & Model | `chaos_core::SceneId` + `chaos_scene::{Scene, SceneState}` — l'identité stable et le modèle | ✅ |
| 2. Ownership & Membership | `chaos_scene::SceneMember` + `Scene::{spawn, adopt, members, contains, unload}` — l'appartenance par composant | ✅ |
| 3. Hierarchy | `chaos_scene::ChildOf` + `hierarchy::{attach, detach, parent_of, children_of, despawn_recursive}` | ✅ |
| 4. Transforms | `chaos_core::GlobalTransform` + `chaos_scene::TransformPropagation` (service moteur, `stages::POST_UPDATE`) | ✅ |
| 5. Lifecycle & Manager | `chaos_scene::SceneManager` — le point d'entrée unique, les transitions gouvernées | ✅ |
| 6. Serialization Format | `chaos_scene::{SceneData, EntityData, FORMAT_VERSION}` — capture/validate/apply, indices de snapshot | ✅ |
| 7. Save/Load & Asset References | `chaos_scene::MeshRef` + encode/decode texte + `chaos_engine::scenes::{save_scene, load_scene}` | ✅ |
| 8. Prefab Foundation | `chaos_scene::Prefab` — capture d'un sous-arbre, instanciation fraîche, racine rendue | ✅ |
| 9. Multi-Scene & Persistent Entities | actives multiples (couches), principale = la plus ancienne, `Scene::release` | ✅ |
| 10. Engine Integration & Runtime Boundary | la démo chargée du fichier committé ; confinement d'erreur prouvé ; la frontière runtime | ✅ |
| 11. Validation finale | Audit statique, cas invalides nommés, verrous CI d'isolation, portes, runs GPU — le Scene System déclaré stable | ✅ |

## SceneId — l'identité, dans le cœur

- **`chaos_core::SceneId`** — dérivé du nom logique par le FNV-1a 64 bits **partagé avec `AssetId`** (même fonction interne, chaque identité verrouillée par ses propres vecteurs de référence). Déterministe à travers sessions, machines et réseau : un serveur et un client calculent la même identité depuis `maps/spawn` sans échange.
- **Pourquoi chaos_core** : le graphe de dépendances — `chaos_network` (réplication de scènes) et `chaos_api` (modding) ne voient que le cœur. L'identité = vocabulaire, le modèle = `chaos_scene`. Le précédent exact d'`AssetId` et d'`Entity`.
- « Distinguée sans dépendre uniquement de son nom ou chemin » : l'identité est une **valeur de première classe** (u64 — clés de maps, paquets, références sérialisées) ; le nom est une métadonnée ; le chemin disque n'existe pas dans le modèle (il appartiendra à la persistance).

## Le modèle de scène — cinq états, les transitions avec leurs machineries

`Scene { id, name, state }` — les métadonnées minimales. `SceneState` nomme le cycle de vie complet exigé par la phase :

| État | Sens |
|---|---|
| `Empty` | créée, aucun contenu — l'état de `Scene::new` |
| `Loaded` | contenu chargé, pas encore active |
| `Active` | la scène qui vit — ses entités sont dans le World |
| `Unloading` | en cours de déchargement |
| `Failed` | invalide ou en échec, inutilisable en l'état |

Le vocabulaire est livré, **les transitions non** : chacune arrivera avec sa machinerie (le chargement avec la persistance, l'activation avec la gestion du cycle de vie) — le patron maison de la préparation sans implémentation. De même, la variante d'erreur scène et le registre de scènes attendront leurs premiers chemins d'échec et de gestion réels.

## Appartenance — un composant, pas une liste

L'appartenance d'une entité à une scène est un **composant ECS** (`SceneMember { scene: SceneId }`), jamais une liste d'entités dans la scène. La relation vit dans le World, la source de vérité unique — les quatre soucis de la spec sont dissous par construction :

- **référence périmée impossible** : le despawn détache tous les composants (cohérence du World, phase 4) — une entité morte disparaît de sa scène automatiquement, rien à synchroniser ;
- **générations et despawn respectés gratuitement** : ce sont ceux du World, rien n'est répliqué ;
- **la distinction scène/global est structurelle** : une entité sans `SceneMember` est globale/persistante — un marqueur `Persistent` pourra affiner quand un besoin réel viendra ;
- **compose avec les Commands** : `SceneId` est Copy — une closure différée insère `SceneMember::new(id)` comme n'importe quel composant.

Les opérations vivent sur `Scene` et empruntent le World (`&mut World` dans chaque signature — la non-possession, visible) : `spawn` (spawn + membership en un geste), `adopt` (revendique une existante ; morte = erreur explicite ; re-domiciliation rend l'ancienne scène), `members` (la requête filtrée), `contains` (générationnel), `unload` (despawne tous ses membres et eux seuls, rend le compte). **Les premières transitions réelles du modèle** : `unload` passe par `Unloading` et revient à `Empty` (scène réutilisable) ; la branche d'échec impossible par construction mène honnêtement à `Failed`, dont `unload` est la voie de récupération. Une scène `Failed` refuse spawn/adopt (erreur explicite). Le spawn ne change pas l'état : `Empty` signifie « rien de chargé » — charger appartient à la persistance, le contenu se requête en vif.

## Hiérarchie — un lien, jamais deux listes

Le parent/enfant est **un composant sur l'enfant** (`ChildOf { parent: Entity }`) et rien d'autre — pas de `Children(Vec)` redondant côté parent : l'état redondant se désynchronise (la leçon des hiérarchies doublement liées, bevy en tête). Les garanties de la spec deviennent structurelles :

- **un seul parent direct** : un composant = une valeur ; le re-attach re-domicilie et **rend l'ancien parent** (le patron d'`adopt`) — jamais silencieux ;
- **plusieurs enfants** : `children_of` est la requête filtrée (le patron de `members`) — aucun ordre de fratrie garanti (viendra si un besoin réel l'exige) ;
- **pas de cycles** : `attach` remonte la chaîne d'ancêtres du parent, O(profondeur), erreur explicite (soi-même compris) ;
- **erreurs explicites** : parent mort (un lien mort-né est indéfendable), enfant mort (la garantie d'écriture du World), cycle.

**Le despawn direct d'un parent, défini** : le World ne connaît pas la hiérarchie (l'ECS ne dépend pas des scènes), donc pas de cascade automatique — les **lectures auto-cicatrisent** : `parent_of` ne rend un parent que vivant (lien mort → l'enfant se lit racine), `children_of(mort)` est vide ; aucune poignée morte ne sort de l'API. Le chemin propre des composites (véhicule → roues) : `despawn_recursive` (collecte du sous-arbre puis despawn, compte rendu).

**Le déchargement, par composition** : `unload` despawne les membres (contrat sous-phase 2, intact), leurs `ChildOf` partent avec eux ; un enfant hors scène survit et se lit racine. Hiérarchie et appartenance sont **orthogonales** — qui doit mourir avec la scène doit en être membre. Structure pure : pas de propagation de transforms (viendra avec ses systèmes), rien du renderer.

## Transforms — local et global

**`Transform` est le LOCAL** : relatif au parent ; relatif au monde pour une racine (le comportement « sans parent » est cette identité même). **`GlobalTransform` est la matrice monde CALCULÉE** — une matrice (`Mat4`) et non un TRS : la composition TRS est perdante (échelle non uniforme du parent + rotation de l'enfant = cisaillement irreprésentable, silencieusement faux) — le choix de Godot et bevy. Il vit dans `chaos_core` : renderer, physique et animation le consommeront et ne voient que le cœur.

- **L'ordre de frame** : les systèmes de jeu mutent les locaux dans `stages::UPDATE` ; la propagation (`TransformPropagation`, un **service moteur** auto-enregistré) calcule les globaux dans `stages::POST_UPDATE` — le deuxième stage arrivé avec son besoin réel ; les subsystems lisent frais.
- **L'algorithme** : remontée de la chaîne d'ancêtres par entité — O(n·profondeur), déterministe, sans ordre de parcours à gérer (et moins cher qu'une descente avec notre stockage). Recalcul complet par frame, assumé ; les dirty flags viendront avec un besoin réel de performance. Deuxième passe : les globaux orphelins (Transform retiré) sont balayés — jamais de global périmé.
- **Les règles** : un ancêtre sans `Transform` contribue l'identité (nœud de groupement) ; un parent mort termine la chaîne (l'auto-cicatrisation composée — l'enfant se propage racine) ; une entité sans `Transform` n'a pas de global.
- **La conservation se choisit par l'opération** : `attach`/`detach` préservent le LOCAL (l'entité saute si les repères diffèrent) ; `attach_keeping_global`/`detach_keeping_global` préservent le GLOBAL (local recalculé `parent⁻¹ × global`, décomposé en TRS — exact sans cisaillement).
- Le renderer reste intact : la démo décompose la matrice globale en TRS pour ses DrawCommands (exact, échelles uniformes) — le renderer apprendra la matrice modèle le jour où un besoin réel l'exigera.

## Cycle de vie — le SceneManager, point d'entrée unique

Le manager POSSÈDE les scènes (registre par `SceneId`, doublon refusé) et détient la POLITIQUE des transitions — les états prennent vie :

| Opération | Exige | Produit |
|---|---|---|
| `create`/`register` | identité libre | `Empty` |
| `load(world, id, populate)` | `Empty` (recharger = décharger d'abord) | `Loaded` — ou `Failed` si populate échoue |
| `activate(id)` | `Loaded` + pas déjà active | `Active`, ajoutée en fin des couches (la première activée est la principale) |
| `deactivate(id)` | active (sinon erreur nommée) | `Loaded` ; désactiver la principale promeut la suivante |
| `unload(world, id)` | PAS active — couches comprises | `Empty` (récupère aussi une `Failed`) |
| `replace(world, id)` | cible `Loaded` | remplace LA PRINCIPALE seule : l'ancienne désactivée PUIS déchargée ; la cible `Active` en tête — les couches ne bougent pas |

- **Charger = remplir depuis une source de contenu** : aujourd'hui une closure (le code de l'app) ; la persistance en fournira une qui lit un fichier — la forme est prête.
- **`Failed` est atteignable par un chemin réel** (populate en échec, contenu partiel conservé) ; la garde spawn/adopt-sur-Failed est effective, `unload` est la récupération.
- **Plusieurs actives — les couches de monde** (monde + interface + intérieur) : `actives()` dans l'ordre d'activation, **la PRINCIPALE = la plus ancienne** (`main()` = `actives[0]`, zéro état supplémentaire, déterministe ; un `set_main` explicite viendra si un besoin réel l'exige). Aucune active = état légitime en lecture ; désactiver une non-active = erreur.
- **L'intégrité de l'état par construction** : seul `&Scene` est exposé (spawn/adopt/members/contains composent en `&self`) ; `unload` — qui exige `&mut Scene` — est réservé au manager. Personne ne décharge l'active dans son dos.
- **Shutdown déterministe** : toutes les scènes déchargées en ordre trié par identité, registre vidé — appelé par le moteur à l'arrêt (`EngineContext`, `world_and_scenes()` pour les opérations à deux emprunts).

## Format de sérialisation — pur données, indices de snapshot

`SceneData` est la représentation persistante : **pur données** (String, u32, f32, Option — tout public et plat, lisible par les outils), indépendante de l'état runtime.

- **Les `Entity` runtime ne persistent JAMAIS** (index+génération = poignées de session) : les entités du snapshot sont un `Vec`, le parent est un **indice dans ce vec** ; à la reconstruction, des entités fraîches sont spawnées et la carte indice→Entity recâble la hiérarchie. Zéro pointeur, zéro handle GPU, zéro détail backend.
- **Le set v1** : le nom (l'identité SOURCE — `SceneId` recalculé, jamais stocké, aucune désync possible), les `Transform`, la hiérarchie en indices, l'appartenance implicite (tout ce qui est capturé est membre). `GlobalTransform` n'est jamais sérialisé : calculé, la propagation le reconstruit. **Les références d'assets** entreront au format avec leurs composants (`AssetId` est né sérialisable) — par montée de version.
- **Capture déterministe** (membres triés par entité — deux captures du même monde sont égales) ; un parent hors du snapshot est capturé racine (la persistance capture la structure INTERNE de la scène).
- **`version` + `validate`** = l'évolutivité sans migration anticipée : version inconnue rejetée (la porte où les migrations futures brancheront), parents hors bornes, auto-parenté, cycles, transforms non finis (le précédent de la porte de validation des assets) — le tout hors de tout World (les outils valideront des fichiers).
- **`apply` valide d'abord** (données invalides → le monde n'est pas touché) et il est volontairement additif : la politique appartient au manager — `apply` est exactement une source `populate` (`manager.load(world, id, |scene, world| data.apply(scene, world))` — la promesse de la sous-phase 5, tenue). L'encodage disque appartient à la persistance.

## Persistance — le disque par le pipeline, la couture au moteur

Trois décisions, zéro nouvelle dépendance :

- **Le chargement passe par l'Asset Pipeline** : un fichier de scène EST un asset (`AssetKind::Scene`, déclaré ; octets par `AssetManager::load_bytes` — l'I/O reste confinée au pipeline). Le PARSING reste dans chaos_scene (pur : texte ↔ `SceneData`, zéro I/O) — le pipeline ignore le format, chaos_scene ignore le disque, le renderer n'est vu de personne.
- **La couture vit dans `chaos_engine::scenes`** (le patron de `chaos_engine::assets`) : `load_scene` = octets → UTF-8 → décodage → validation → **résolution des références** (chaque mesh doit être connu du registre — référence cassée = erreur explicite nommant l'entité et l'identité) ; `save_scene` = encodage + écriture (un acte d'AUTORING — le pipeline reste fournisseur, l'éditeur héritera de ce chemin).
- **`MeshRef { mesh: AssetId }`** — le premier composant porteur de références d'assets : identité stable, jamais un handle GPU. Les meshes **procéduraux** reçoivent la leur via `AssetSource::Procedural` : le pipeline est l'espace de noms de résolution, même pour le contenu généré. L'association material reste une politique d'app (le format de material est un futur documenté).

**Le format texte** (`.cscn`), fait maison (le précédent PPM/base64) — versionné, déterministe (hash stable entre deux runs), lisible :

```
chaos-scene 1
name scenes/demo
entity
transform tx ty tz rx ry rz rw sx sy sz
mesh 40eb6ab7ce746fce
parent 1
```

Parseur strict, malformations nommées ; flottants en représentation la plus courte qui reboucle bit-exact. **La démo est la preuve vivante** : à chaque lancement, sa scène est construite, capturée, sauvée dans `assets/scenes/demo.cscn`, déchargée puis rechargée DEPUIS LE FICHIER — ce qui s'affiche est toujours la scène du disque (`Spin`, comportement et non données, est ré-attaché après chargement — le futur scripting).

## Prefabs — la fondation

Un prefab est un **sous-arbre réutilisable dans LE format de scène** (`EntityData`, indices de snapshot) — sans identité de scène ni version. Rien d'inventé : la fondation est une réutilisation.

- **Capture** (`Prefab::capture(name, world, root)`) : BFS parents-avant-enfants (les parents pointent toujours en arrière — structure auto-cohérente) ; les liens EXTERNES sont coupés (la racine du prefab est racine même si l'entité capturée avait un parent) ; l'appartenance de scène n'est pas capturée — le prefab est indépendant des scènes.
- **Instanciation** (`instantiate(scene, world) -> Entity`) : valide d'abord (règles partagées du format + non-vide + racine unique à l'indice 0) ; des entités **FRAÎCHES**, membres de la scène cible ; composants et hiérarchie restaurés ; **la racine rendue** — le placement est un geste de l'appelant (`world.insert(root, transform)`, la propagation déplace toute l'instance).
- **La séparation asset/instances est structurelle** : le `Prefab` est de la donnée, les instances des entités du World — deux instanciations ne partagent RIEN (vérifié : ensembles d'entités disjoints, mutation d'une instance sans effet sur l'autre). L'unload de la scène cible emporte toutes les instances (la composition appartenance × prefab).
- Exclusions explicites (les futurs de l'éditeur) : overrides, variantes, imbrication avancée, synchronisation asset↔instances. Le fichier prefab réutilisera l'encodage texte le jour venu ; la composition d'instances passe déjà par `hierarchy::attach`.

## Multi-scènes et persistance — la fondation

L'architecture ne suppose plus un monde monolithique — sans devenir du streaming :

- **Coexistence et isolation structurelles** : les scènes coexistent dans le registre, leurs entités sont isolées par `SceneMember` (l'unload emporte SES membres et eux seuls — prouvé avec deux scènes actives) ; le `World` reste unique et seul propriétaire des données vivantes — multi-SCÈNES, pas multi-mondes.
- **Préserver = `Scene::release`** : un membre libéré (sans despawn) devient global/persistant — il survit aux déchargements. **Transférer = `Scene::adopt`** (existant — rend l'ancienne scène). Le flux complet : release → globale → adopt ailleurs. Pas de marqueur `Persistent` dédié : l'absence de `SceneMember` EST la persistance (la doctrine de la sous-phase 2).
- Niveaux additionnels, interfaces, intérieurs et mondes découpés sont prêts à arriver comme des couches actives ; le **world streaming** (cellules, distances, chargement asynchrone) reste explicitement hors périmètre.

## Intégration moteur et frontière runtime

Le Scene System s'est branché **en bloc**, sans réécrire l'Engine Core — la carte des branchements :

| Pilier | Branchement |
|---|---|
| `EngineContext` | porte le `SceneManager` (`scenes()`/`scenes_mut()`/`world_and_scenes()`) |
| `Subsystem` | les subsystems consomment les scènes par le contexte (la démo en est le modèle) |
| Scheduler ECS | les systèmes mutent en `UPDATE`, la propagation calcule en `POST_UPDATE`, les subsystems lisent frais — l'ordre garanti et testé |
| Asset Manager | les fichiers de scène sont des assets (`AssetKind::Scene`, octets par le pipeline, références résolues au chargement) |
| Renderer | consomme `GlobalTransform` + `MeshRef` via l'extraction de l'app — jamais de couplage (verrous CI) |
| Shutdown | les scènes détruites en ordre déterministe, garanti par le moteur |

- **Le confinement d'erreur, prouvé au niveau moteur** : un chargement en échec ne laisse rien de partiel — monde vide, manager cohérent, le moteur continue de tourner (testé sur le chemin complet contexte + pipeline). Un populate en échec est contenu dans sa scène (`Failed`, récupérable).
- **L'extraction de rendu est un patron côté app** : la table de résolution AssetId → handles (dont l'association material) est une politique d'app — elle se généralisera avec le format de material, pas avant.
- **La frontière runtime** : le moteur offre le MÉCANISME (scènes, cycle de vie, persistance, prefabs) ; les sessions, gamemodes et l'orchestration appartiennent au futur Runtime Platform (`chaos_runtime`), au-dessus. Aucune logique de jeu ni de session dans le moteur.
- **La démo est du contenu** : elle charge `assets/scenes/demo.cscn` (fichier committé) et n'assemble AUCUNE entité en code — le checkpoint de la phase, vivant à chaque lancement.

## Ce que les sous-phases suivantes brancheront ici

- **Persistance** : sérialisation, sauvegarde, chargement — les états `Loaded`/`Failed` prennent vie.
- **Cycle de vie** : activation, déchargement propre — `Active`/`Unloading` prennent vie.
- **Prefabs et préparation éditeur** : instanciation répétable, description exploitable par Chaos Editor.
