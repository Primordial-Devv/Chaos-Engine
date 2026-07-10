# ECS — architecture

Référence des choix de la phase 4. Le principe directeur : **l'ECS est le cœur logique du moteur** — le renderer affiche, l'Asset Pipeline fournit, l'ECS fait vivre les objets du monde. Simple, robuste, pensé long terme : c'est une fondation, pas un détail d'implémentation.

**Statut : sous-système STABLE** (phase 4 validée). Le moteur ne manipule plus seulement des ressources et des objets graphiques : il manipule un monde vivant — entités, composants, systèmes. Les phases suivantes (Scene System, Physics, Animation, Audio, Runtime, Editor, Networking…) viennent s'appuyer dessus **sans refonte de son architecture**.

## Les règles de la phase

- Le monde = **entités** (identités pures) + **composants** (leurs données) + **systèmes** (leur comportement) — chaque concept arrive avec sa sous-phase, rien n'est anticipé.
- L'ECS ne connaît que `chaos_core` — jamais le renderer, jamais l'Asset Pipeline (il produit des données qu'ils consomment/référencent).
- Aucune logique gameplay : le gameplay appartiendra à la plateforme (runtime/scripting), au-dessus.
- Pas de journalisation sur les chemins chauds (spawn/despawn, itérations) — l'observabilité passe par les compteurs et le futur éditeur.

## Feuille de route Phase 4

| Sous-phase | Destination | Statut |
|---|---|---|
| 1. Entities | `chaos_core::Entity` (identité générationnelle) + `chaos_ecs::Entities` (l'allocateur) | ✅ |
| 2. Components | `chaos_ecs::{Component, ComponentStorage}` — le trait opt-in + le sparse set générationnel | ✅ |
| 3. World | `chaos_ecs::World` — le conteneur central : allocateur + registre des storages, la cohérence entités ↔ composants | ✅ |
| 4. Resources | `chaos_ecs::{Resource, Resources}` — les données globales sans entité, portées par le World | ✅ |
| 5. Systems | `chaos_ecs::{System, Systems}` — les traitements, jamais propriétaires ; le registre ordonné, hors du World | ✅ |
| 6. Scheduler | `chaos_ecs::Schedule` — les stages nommés : l'organisation déclarée de l'exécution | ✅ |
| 7. Queries | `World::{query, query_mut, query2, query2_mut}` — l'itération dense exposée, la jointure par sondage | ✅ |
| 8. Events & Commands | `chaos_ecs::{Message, Messages, Commands}` — les files de messages typées + les modifications différées | ✅ |
| 9. Intégration moteur | `EngineContext::{world, schedule}` — le moteur tient et tick l'ECS ; isolation verrouillée en CI | ✅ |
| 10. Validation finale | Audit statique, portes, runs GPU réels, docs — l'ECS déclaré stable | ✅ |

## Entity — l'identité pure

Une Entity **n'est qu'une identité** : pas de logique, pas de comportement, pas de données. Tout objet du monde commence par elle.

- **`chaos_core::Entity`** — `index` + `generation`, opaque en pratique (`Copy`, `Eq`, `Hash`, `Ord`, Display `entity:{index}v{génération}`). **Pourquoi core** : le graphe de dépendances — `chaos_network` (réplication) et `chaos_api` (la surface de modding) ne voient que core et manipuleront des entités partout. L'identité = vocabulaire ; la machinerie = `chaos_ecs`. Le précédent exact d'`AssetId`.
- **Générationnelle** — le patron maison des pools du renderer : un slot recyclé change de génération, une Entity détruite est détectée à jamais, jamais résolue vers une autre. C'est la stabilité de la représentation.
- **`from_raw` est public et sûr par construction** : les systèmes du moteur (sérialisation, réseau) en ont besoin, et une Entity forgée est inoffensive — l'allocateur la rejette (`is_alive` faux, `despawn` en erreur explicite).
- **`chaos_ecs::Entities`** — la seule fabrique d'Entity valides : `spawn` (recyclage des slots libres, échec d'épuisement u32 théorique mais explicite — jamais de panic), `despawn` (morte/périmée/forgée → erreur explicite), `is_alive`, `len`/`is_empty`, `iter` (les vivantes — le socle des futures requêtes, du debug, de l'éditeur).

## Composants — les données, jamais le comportement

Un composant est de la **donnée pure** attachée à une entité — le comportement appartiendra aux systèmes.

- **`Component`** : un marqueur opt-in (`impl Component for MonType {}`) — implémenter documente l'intention. La contrainte `Send + Sync + 'static` prépare le parallélisme futur **par contrainte** (gratuite aujourd'hui), pas par machinerie. `Transform` est le premier composant réel ; `GlobalTransform` est arrivé AVEC son système (`TransformPropagation`, phase 5) — la règle en action ; Mesh/Material (références d'assets), Camera, Light, Audio, Physics, Script arriveront **chacun avec son système** — l'extensibilité, c'est le trait, pas une liste anticipée.
- **`ComponentStorage<T>` — le sparse set générationnel.** La décision de stockage argumentée :
  - pas le `HashMap` naïf — l'ECS vit d'itérations, elles doivent être denses ;
  - pas les archétypes (bevy/flecs) — sophistication inutile en V1 (déplacements de tables, invalidations, complexité énorme) pour un gain qui ne compte qu'à très grande échelle ; **évolution possible plus tard sans changer le contrat public** ;
  - le sparse set (le choix EnTT) : accès O(1), insertion/retrait O(1) (`swap_remove` + correction du sparse), **données contiguës** pour l'itération.
- **Sûreté générationnelle en lecture, inconditionnelle** : l'entrée dense porte l'Entity complète — une entité périmée n'est jamais résolue vers les données d'une autre. Le storage ne connaît pas la vivacité : c'est le World (sous-phase suivante) qui garantit de n'écrire que pour des vivantes ; `insert` rend toujours la valeur délogée (jamais de drop silencieux).
- Pas de journalisation sur ces chemins chauds (règle de la phase).

## World — le conteneur central, la cohérence

Le World unit l'allocateur et les storages (un par type de composant, registre par `TypeId` — le `'static` du trait `Component` paie ici). Tous les systèmes futurs travailleront dessus. Il porte la cohérence que les briques seules n'ont pas :

- **La vivacité à l'écriture, tenue** : `insert` refuse une entité morte, périmée ou forgée avec une erreur explicite — la promesse documentée en sous-phase 2 (« le World garantit de n'écrire que pour des vivantes ») a désormais son gardien.
- **Pas de données orphelines** : `despawn` détache les composants de TOUS les storages — un index recyclé démarre propre, la mémoire des composants est rendue à la destruction, et la sûreté générationnelle du storage reste la seconde ligne de défense en lecture. L'ordre compte : la destruction est validée AVANT tout détachement — un `despawn` avec une poignée périmée erre sans toucher l'occupant actuel de l'index.
- **La doctrine des retours** : lecture et détachement en `Option` (le périmé est inoffensif par construction — il ne résout jamais), écriture en erreur (`ChaosError::Ecs` — écrire pour un mort est un bug de logique, nommé).
- **API** : `spawn`/`despawn`/`is_alive`/`len`/`is_empty`/`iter` (les vivantes) côté entités ; `insert` (rend la valeur remplacée)/`get`/`get_mut`/`remove` côté composants — les mêmes noms que `ComponentStorage`, un niveau au-dessus.
- **`Send + Sync` par construction** (verrouillé par test) : la contrainte du trait `Component` paie une deuxième fois — le parallélisme futur reste préparé par contrainte, pas par machinerie.

## Ressources — les données globales, sans entité

Certaines données du monde n'appartiennent à aucune entité : le temps global, les paramètres moteur, la configuration, l'état global. Les forcer dans une « entité singleton » serait un mensonge de modèle — d'où un registre distinct, **au plus une valeur par type**.

- **`Resource`** : le miroir exact de `Component` — un marqueur opt-in (`impl Resource for MonType {}`), des données jamais du comportement, `Send + Sync + 'static` par contrainte. Un même type peut implémenter les deux traits : les deux registres sont étanches (verrouillé par test).
- **`Resources`** : le même mécanisme type-erased que les storages du World (clé `TypeId`), sans la dimension entité — aucune opération ne traverse tous les types, donc pas de trait pont : le `Box<dyn Any + Send + Sync>` nu suffit. `insert` rend la valeur remplacée (jamais de drop silencieux) ; même la branche impossible du `remove` réinsère au lieu de détruire.
- **Le World les porte** : `insert_resource`/`resource`/`resource_mut`/`remove_resource` — les systèmes futurs y liront le temps et les paramètres. Tout en `Option`, pas d'erreur : les ressources n'ont pas de vivacité, remplacer est légitime.
- **`Time` est la première ressource réelle** — le « temps global ». La frontière : `Time` (l'instantané, données pures) entre dans le monde ; `FrameClock` (la machinerie qui possède l'`Instant` et le clamp) reste au moteur. La donnée entre dans l'ECS, la machinerie jamais. Le branchement moteur → resource `Time` à chaque frame viendra avec la sous-phase d'intégration.

## Systèmes — le comportement, enfin, mais jamais les données

Un système est un **traitement appliqué aux composants**. Les deux lois de la spec sont des contraintes de conception, pas des vœux :

- **« Jamais posséder les données »** — `run(&self, world: &mut World)` : le `&self` interdit tout état mutable dans le système **par signature**. L'état d'un système, s'il en faut un, est une ressource du World (inspectable, sérialisable plus tard) — jamais un champ privé.
- **« Ne faire que transformer l'état du monde »** — un système ne reçoit QUE `&mut World`, et le registre `Systems` vit **hors** du World : les traitements ne vivent pas dans les données (et l'auto-emprunt est évité par construction). Le moteur tiendra un World et un Systems, côte à côte.
- **`Systems`** — la famille est complète : `Entities`, `Resources`, `Systems`. Exécution **séquentielle dans l'ordre d'enregistrement, déterministe** ; le parallélisme futur viendra des contraintes déjà posées (`Send + Sync` partout), pas d'une machinerie anticipée (pas de stages, pas de priorités, pas de graphe de dépendances — quand un besoin réel arrivera). Doublon de nom rejeté en nommant l'existant (le précédent AssetRegistry). L'écho exact du patron `chaos_engine::Subsystem`, à l'échelle ECS.
- **La première erreur arrête le tick** — un monde en état inconnu ne doit pas continuer ; l'erreur nomme le système fautif, les mutations faites avant l'échec restent (pas de rollback magique).
- Aucun système de production n'est livré ici : les systèmes réels (transform, caméra…) arriveront **avec leurs features**. L'ergonomie d'origine des corps — `collect()` des entités puis muter — était la limitation assumée : les requêtes (sous-phase 7) l'ont résolue.

## Ordonnancement — l'organisation déclarée

Dès que le moteur a plusieurs familles de traitements, l'ordre global doit venir d'une **organisation déclarée**, pas de l'ordre accidentel des appels d'enregistrement. Le `Schedule` : des **stages nommés**, déclarés dans l'ordre d'exécution.

- **Le Schedule compose des `Systems`** — responsabilité unique : le registre plat reste l'unité d'exécution (ordre d'enregistrement, arrêt à la première erreur, doublons rejetés), le Schedule n'ajoute QUE l'organisation. Zéro modification de l'existant.
- **Les stages préparent les trois futurs sans en implémenter aucun** :
  - *parallélisme* — la frontière de stage est le futur point de synchronisation : un jour les systèmes d'un même stage se paralléliseront (les contraintes `Send + Sync` sont déjà partout) ;
  - *dépendances* — « A avant B » s'exprime à gros grain par des stages différents ; le graphe fin intra-stage pourra arriver **sans changer le contrat public** ;
  - *phases moteur* — les stages sont le **mécanisme**, les noms de phases sont la **politique** : elle appartient au moteur (intégration), pas à chaos_ecs — aucune enum de phases codée en dur.
- **Unicité des noms PAR stage** (déléguée à `Systems::add`) : le même nom peut vivre dans deux stages — un système de synchronisation peut légitimement tourner deux fois ; l'erreur d'exécution nomme le stage, aucune ambiguïté (« stage 'x' failed: system 'y' failed: … »).
- Pas de `add_stage_before/after` : ce besoin (plugins tiers) appartient à la plateforme, bien plus tard — limitation assumée.

## Requêtes — demander seulement ce dont on a besoin

Les requêtes n'inventent rien : elles **exposent** l'itération dense que le sparse set porte depuis la sous-phase 2 — le choix de stockage paie sa promesse. Le coût suit les correspondances, jamais la taille du monde.

- **La famille** : `query::<A>()` / `query_mut::<A>()` — les paires (Entity, &A)/(Entity, &mut A), coût **O(|A|)** ; `query2::<A, B>()` / `query2_mut::<A, B>()` — la jointure (Entity, &A, &B), avec mutation du meneur pour la variante `_mut`.
- **A mène, B est sondé** : le meneur est itéré densément, l'autre sondé en O(1) générationnel — coût O(|A|). La règle est la main du développeur : *mettez le composant le plus rare en premier*. Storage absent → requête vide, jamais une erreur.
- **Zéro unsafe** : les jointures mutables des ECS classiques reposent sur de l'unsafe d'aliasing ; ici le split de deux storages vient de std (`HashMap::get_disjoint_mut` — deux `&mut` disjoints prouvés par les clés, le sondé rétrogradé en partagé). `query2_mut::<T, T>` est une erreur explicite, jamais un panic.
- **Les lectures se composent** : jointure `query2::<A, B>` + sonde `world.get::<C>(entity)` dans la boucle — les emprunts partagés coexistent. `&mut A + &mut B` : aucun besoin actuel, différé.
- Pas de filtres (With/Without/Changed), pas de tuples type-level, pas de cache de requêtes : quand un besoin réel arrivera.

## Communication — messages et commandes différées

Les deux premiers mécanismes de communication interne, explicitement préparatoires.

- **`Message` et non `Event`** — la décision de nommage : `chaos_core::Event` existe (l'enum fenêtre/input de tout `Subsystem::on_event`) ; un trait `Event` dans l'ECS créerait une collision permanente dans chaos_engine. Les événements de l'ECS sont des **messages typés en file** — et `chaos_core::Event` est le premier message réel : exactement ce que l'intégration pompera dans le monde.
- **`Messages`** — une file FIFO par type (clé `TypeId`), **auto-créée à la première émission** (le précédent de l'insert composant : zéro cérémonie). L'ordre d'émission est l'ordre de lecture — déterminisme. `read` (partagé), `drain` (consommation exclusive, `mem::take`), et `clear()` qui balaye **toutes** les files d'un coup via le pont type-erased (le patron `AnyStorage`) : c'est la primitive du balayage de frame — le mécanisme vit ici, **la politique de durée de vie appartient à la boucle moteur** (intégration). Le World les porte : `send_message`/`messages`/`drain_messages`/`clear_messages`.
- **`Commands`** — les modifications du monde enregistrées, appliquées à un point sûr. La forme closure (`FnOnce(&mut World) -> ChaosResult<()>`) dissout la référence en avant : un spawn différé compose librement dans son différé. Sucres : `despawn`/`insert`/`remove` (l'insert différé abandonne la valeur remplacée — il ne peut la rendre à personne).
- **Les Commands vivent HORS du World** — sinon impossible d'enregistrer pendant qu'une requête emprunte le monde. Le patron local : buffer local pendant l'itération (emprunt partagé), `apply` après. Le patron cross-système : `impl Resource for Commands` — un système tôt enregistre dans la ressource, un « flush » tardif applique. Les points d'application automatiques aux frontières de stages viendront avec le parallélisme — préparés, pas implémentés.
- **`apply` est strict** : FIFO, les règles du World telles quelles au moment de l'apply, la première erreur arrête en nommant l'index, les mutations antérieures restent, la file est consommée dans tous les cas. La tolérance se compose dans la closure de l'appelant — sa politique.

## Intégration moteur — la couture par le contexte

L'ECS entre dans le moteur par le patron maison de la couture : **chaos_engine est la seule crate qui voit tout**, et le World + le Schedule entrent dans `EngineContext` comme l'Asset Manager avant eux (`world()`/`world_mut()`/`schedule_mut()`).

- **L'ordre de frame** (voir `docs/architecture/engine-loop.md`) : tick du clock → ressource `Time` rafraîchie → **le Schedule tourne AVANT les updates des subsystems** (ils lisent l'état frais que les systèmes viennent d'écrire) → updates → `clear_messages()` — **un message vit un update**. Les événements fenêtre/input sont pompés en messages à l'arrivée (`impl Message for chaos_core::Event` paie ici). Un schedule en échec = `error!` + arrêt propre : un monde en état inconnu ne continue pas.
- **La politique des phases vit au moteur** : `chaos_engine::stages::UPDATE` — un seul stage réel aujourd'hui, déclaré au démarrage ; les suivants arriveront avec leurs besoins (le mécanisme est prêt depuis la sous-phase 6).
- **L'isolation, verrouillée en CI** (`chaos_ecs/tests/isolation.rs`, le précédent wgpu) : les sources et manifestes du renderer et de l'Asset Pipeline ne mentionnent jamais chaos_ecs ; le manifeste de chaos_ecs ne dépend d'aucune crate moteur autre que chaos_core.
- **Le premier système de production vit dans l'app** : le cube central de la démo sandbox est une entité (`Transform` + composant local `Spin`), animée par le système `demo.spin` enregistré dans `stages::UPDATE` — le contenu définit ses composants et systèmes, le moteur fournit le mécanisme. L'histoire de la plateforme moddable, en miniature. Visuel inchangé : même mathématique, nouvelle plomberie.

## Ce que les phases suivantes brancheront ici

- Hiérarchie parent/enfant (Scene System — `chaos_scene` est l'unique dépendance latérale autorisée vers chaos_ecs), réplication (network), inspection (éditeur), composants de rendu (mesh/material) avec leurs systèmes d'extraction.
