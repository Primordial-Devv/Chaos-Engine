# Asset Pipeline — architecture

Référence des choix de la phase 3. Le principe directeur : **l'Asset Pipeline produit les ressources, le reste du moteur les consomme** — le renderer en premier, puis l'ECS, les scènes, l'éditeur, la plateforme de modding.

## Les règles de la phase

- L'Asset Pipeline est un **service du moteur** : il ne dépend jamais de l'éditeur, jamais du gameplay — il gère des ressources, rien d'autre.
- **Aucun chargement disque sauvage** : le moteur ne lit jamais un fichier « au hasard » ; toute ressource passe par le pipeline.
- Le **renderer n'est plus modifié** : il consomme des ressources déjà préparées (descripteurs, octets décodés) via son API publique — aucune logique de format (glTF, PNG…) n'entre dans `chaos_renderer`.
- Vocabulaire partagé dans `chaos_core`, machinerie dans `chaos_assets` (le patron Transform/Camera : le concept est commun, le système est confiné).

## Feuille de route Phase 3

| Sous-phase | Destination | Statut |
|---|---|---|
| 1. Asset Identity | `chaos_core::asset::AssetId` — identité stable dérivée du nom logique (FNV-1a 64 verrouillé) | ✅ |
| 2. Asset Registry | `chaos_assets::registry` — le catalogue central : existence, provenance, type, état | ✅ |
| 3. Asset Manager | `chaos_assets::manager` — le point d'entrée unique : cycle de vie complet, l'I/O de ressources confinée ici | ✅ |
| 4. Asset Importers | `chaos_assets::{import, importers}` — architecture d'import extensible (trait + routage kind/extension), builtins WGSL et PPM | ✅ |
| 5. glTF / GLB | `importers/gltf.rs` — le format 3D de référence (crate `gltf` taillée, GLB + `.gltf` auto-suffisant, première primitive TRIANGLES → `MeshData`) | ✅ |
| 6. Cache & Lifetime | rétention (`acquire`/`release`), `unload` protégé, `evict_unused` — la mutualisation et le crochet du streaming | ✅ |
| 7. Erreurs & Validation | la porte de validation sémantique (`ImportedAsset::validate`) appliquée à tout import — aucun asset invalide n'atteint les consommateurs | ✅ |
| 8. Préparation du Hot Reload | les deux primitives : `reload(id)` (remplacement sous identité, donnée précédente conservée en cas d'échec) + version de contenu (invalidation pull) | ✅ |
| 9. Validation | la couture livrée (`chaos_engine::assets`, service `EngineContext`, démo nourrie par fichiers) + audit complet | ✅ |

**Phase 3 Asset Pipeline : 9/9 — complet.** Le pipeline est un pilier du moteur : identité stable, catalogue central, I/O confinée, importeurs extensibles et validés (PPM/WGSL/glTF), durée de vie gouvernée, primitives de hot reload, et distribution au renderer par la couture — le Scene System, l'ECS et l'Editor s'appuieront dessus.

## Identité — `AssetId`

Toute ressource du moteur possède une identité **stable, unique et indépendante des chemins de fichiers** :

- **`AssetId`** (`chaos_core::asset`) : 64 bits opaques, `Copy`/`Eq`/`Hash`/`Ord`, dérivés du **nom logique** par FNV-1a 64 — implémenté dans le moteur, zéro dépendance.
- **Déterministe à travers les sessions et les machines** : même nom → même identité. C'est ce qui rend l'identité sérialisable dans les scènes, transmissible sur le réseau (un serveur désigne un asset à ses clients) et stable pour le contenu moddé.
- **L'algorithme est verrouillé par des vecteurs de référence en test** (même philosophie que les conventions mathématiques) : une dérive silencieuse invaliderait toute référence sérialisée.
- **Le nom logique est virtuel**, jamais un chemin OS. Convention : minuscules, séparateur `/`, sans extension — ex. `textures/brick`, `models/crate`. Le hachage est octet-exact (pas de normalisation : elle appartiendra à l'importeur, qui mappe les fichiers réels vers les noms logiques). Les labels du renderer (`chaos.white`, clés du cache de textures) sont déjà des noms logiques — la convergence est naturelle.
- **Pourquoi dans `chaos_core`** : les sous-systèmes ne dépendent que de core (règle du graphe), et `chaos_api` — la surface de modding — ne voit que core. L'identité est du vocabulaire ; le pipeline (`chaos_assets`) est la machinerie.

## Registre — `AssetRegistry`

Le catalogue central (`chaos_assets::registry`) — la référence du moteur : quelles ressources existent, où elles se trouvent, leur type, leur état. **Il catalogue, il ne charge rien** : l'I/O appartient au loader (sous-phase suivante).

- **`register(name, kind, source) → AssetId`** — l'identité est dérivée du nom (`AssetId::from_name`, sous-phase 1). Un id déjà pris est une **erreur explicite qui nomme l'entrée existante** : le même chemin couvre le doublon et la collision de hachage théorique — détectée, jamais silencieuse.
- **`AssetKind`** (`Texture`, `Mesh`, `Material`, `Shader`) : les types que le moteur sait consommer aujourd'hui — audio, scènes, etc. s'ajouteront avec leurs sous-systèmes.
- **`AssetSource`** (`File(PathBuf)` | `Procedural`) : la provenance, **déclarative** — le registre ne lit jamais le disque ; les chemins sont des données que l'importeur consommera.
- **`AssetState`** (`Unloaded` | `Loaded` | `Failed(raison)`) : le cycle de vie. Les transitions (`mark_loaded`, `mark_failed`) sont le contrat du futur loader — id inconnu → erreur explicite. Pas de suppression V1 : le déchargement (mods) viendra avec la plateforme.
- **`AssetEntry`** en lecture seule hors du registre (accesseurs) : toute mutation passe par le registre. `lookup(name)` fournit le mapping inverse vivant ; `iter()` liste tout (futur éditeur, debug).
- Erreurs du domaine : `ChaosError::Asset` (nouvelle variante, patron une-variante-par-domaine).

## Manager — `AssetManager`

Le gardien de la vie des assets et **l'unique point d'entrée** pour demander une ressource (`chaos_assets::manager`) — le cycle complet : **déclarer → charger → servir → décharger**.

- **L'I/O de ressources est confinée ici** : `load_bytes` est le seul endroit du moteur qui lit un fichier de ressource — le parallèle exact de wgpu confiné au backend. Le reste du moteur ne touche jamais le disque.
- **`declare(name, kind, source)`** enregistre au registre (que le Manager possède ; consultation en lecture via `registry()`).
- **`load_bytes(id)`** : cache d'abord (zéro I/O en rechargement), sinon lecture de la source — succès → `Loaded`, échec I/O → `Failed(raison)` **et** erreur explicite ; un asset `Procedural` ne se charge pas (il est matérialisé par son créateur — erreur explicite, état intact) ; id inconnu → erreur explicite. `bytes(id)` : accès cache pur.
- **`unload(id)`** : octets libérés, état → `Unloaded` (idempotent ; décharger un `Failed` réarme le rechargement) — recharger relit la source.
- **V1 sert des octets bruts** : les importeurs typés (image → `TextureDescriptor`, glTF → géométries) s'appuieront sur `load_bytes` sans le remplacer — d'où le nom honnête de l'API.
- **Câblage `EngineContext` différé** au premier consommateur réel (l'importeur qui nourrira la démo) — règle maison constante.

## Importeurs — l'architecture d'import extensible

Octets bruts → importeur → **ressource préparée** (`ImportedAsset`). Le contrat (`chaos_assets::import`) :

- **`AssetImporter`** : un importeur décode UN kind pour UNE famille d'extensions (`kind()`, `extensions()` — minuscules, `import(name, bytes)`). Le Manager route par **kind ET extension** de la source ; aucun importeur → erreur nommée + état `Failed`. L'extensibilité = `register_importer` (formats supplémentaires, kinds futurs, contenu moddé).
- **Les données produites sont neutres** — `TextureData` (RGBA8, rangées serrées, origine haut-gauche), texte WGSL… — jamais des descripteurs du renderer : `chaos_assets` ne dépend pas de `chaos_renderer` (pas de latérales). La **couture** données neutres → `TextureDescriptor` se fait au-dessus, par qui voit les deux — le patron raw-window-handle. Le renderer reste sans aucune logique de format.
- **Builtins** (patron `with_builtins`) : `WgslImporter` (Shader — UTF-8 validé) et `PpmImporter` (Texture — P6 binaire maxval 255, std pur, chaque malformation nommée). Le PNG et sa décision de dépendance appartiennent à la sous-phase texture dédiée ; glTF à la sous-phase mesh.
- **`ImportedAsset` grandit avec les kinds** : Mesh et Material arriveront avec leurs formats ; animations, audio, scripts et scènes avec leurs sous-systèmes — tous par ce chemin.
- `import(id)` met la ressource préparée en cache (`imported(id)` : accès pur) ; échec de décodage → `Failed(raison)` consultable ; `unload` libère aussi l'import.

### La porte de validation sémantique

Les données importées **portent leurs règles de cohérence** (le patron `TextureDescriptor::validate()` du renderer), et le Manager les applique à **tout** import — builtin ou enregistré : c'est la protection du point d'extension (un importeur moddé ne peut pas faire entrer de données incohérentes). Échec → `Failed(raison)` + erreur nommée, l'asset n'atteint jamais un consommateur.

- **`TextureData::validate`** : dimensions non nulles, pixels exactement aux dimensions (RGBA).
- **`MeshData::validate`** : positions non vides et **finies** (NaN/infini rejetés, UV comprises), UV appariées, indices non vides, multiples de 3, et **tous dans les bornes** — jamais de lecture hors limites côté GPU.
- **Frontière shader assumée** : un WGSL invalide est déjà refusé proprement par le renderer (naga en CI pour les builtins, error scope à la création du pipeline) — dupliquer naga dans le pipeline coûterait une dépendance lourde pour une détection à peine plus précoce. Décision documentée.

## glTF — le format 3D de référence

Le glTF est le format officiel des assets 3D de Chaos Engine — nativement main droite / +Y haut / -Z avant : la promesse de `math-conventions.md` se réalise, **zéro conversion de repère à l'import**.

- **La première dépendance de parsing du moteur** : la crate `gltf` (parseur de référence de l'écosystème), **taillée** — `default-features = false, features = ["utils"]`, sans le feature `import` (qui tirerait `image`/`base64` : le décodage d'images reste une décision séparée). Un format officiel mérite le parseur officiel ; le parser à la main (JSON + conteneur + spec des accessors) serait un projet en soi.
- **GLB et `.gltf` auto-suffisant** (extensions `glb`, `gltf`) : les buffers viennent du chunk binaire GLB ou des **data URIs base64** embarquées dans le JSON (décodeur RFC 4648 maison, std pur, verrouillé par vecteurs de référence) — les deux formes collent au contrat des importeurs (des octets, pas d'accès disque). Le `.gltf` multi-fichiers (`.bin` externes) exigera la résolution de dépendances entre assets — erreur explicite nommant l'URI en attendant.
- **Périmètre V1** : première primitive TRIANGLES du premier mesh — positions, UV (zéros si absentes), indices (séquence générée si non indexé). Produit un `MeshData` neutre (indices u32 — la limite u16 du renderer appartient à la couture). Chaque écart = erreur nommée (pas de mesh, pas de TRIANGLES, buffers externes, glTF invalide).
- **Ce que les phases suivantes liront en plus** : normales/tangentes (avec le vertex éclairé — carte lighting), matériaux glTF (sous-phase material), multi-mesh/primitives et hiérarchie de nœuds (Scene System), animations (leur sous-système).

## Cache & durée de vie

Le modèle : **réchauffer ≠ posséder**.

- `load_bytes` / `import` **réchauffent** les caches — jamais de rechargement inutile (prouvé par tests : le fichier peut disparaître, l'asset reste servi) — sans impliquer de possession.
- **`acquire(id)`** = importer + posséder : la rétention compte les consommateurs (la **mutualisation** — le même asset sert N usages, prouvé). **`release(id)`** rend la possession (asset non retenu → erreur explicite) ; le cache reste chaud après release — la libération effective appartient à l'éviction. `retain_count(id)` : l'observabilité (debug, futur éditeur, budgets).
- **`unload(id)` est protégé** : un asset encore retenu refuse de se décharger (erreur avec le compte) — un consommateur ne peut plus casser la mutualisation.
- **`evict_unused()`** libère tout ce que personne ne retient (état → `Unloaded`, rechargement à l'accès suivant) — **le crochet du streaming** : sous pression mémoire, on évince l'inutilisé ; le cycle acquire → release → evict → reload est testé de bout en bout.
- La rétention est **orthogonale à l'état** (état = cycle de chargement, rétention = possession) et gouverne les caches **CPU** du pipeline ; les ressources GPU créées par le renderer ont leur propre vie (handles générationnels, refcount wgpu) — le pont viendra avec la couture asset → GPU.
- Futurs notés, pas commencés : budgets mémoire, politique LRU, streaming réel, guards RAII.

## Préparation du hot reload

Rien du hot reload n'est implémenté (pas de watcher, pas de re-dérivation automatique) — mais **les deux primitives structurelles** qui le rendront possible sans refonte existent et sont testées :

- **`reload(id)`** — le remplacement **sous la même identité, rétention intacte** : la scène peut tenir l'asset pendant qu'il se recharge. Si le rechargement échoue (fichier cassé sauvegardé), **la donnée précédente est conservée et servie** — la scène continue, l'échec est consultable (`Failed(raison)`). La porte de validation s'applique au rechargement comme à l'import. Un futur watcher appellera cette primitive ; un outil de dev peut l'appeler dès aujourd'hui.
- **La version de contenu** (`AssetEntry::version`) — 0 = jamais matérialisé, +1 à chaque (re)matérialisation (`mark_loaded`). La règle des consommateurs de données dérivées (la couture qui tiendra des ressources GPU) : **re-dériver si la version a changé ET que l'état est `Loaded`**. Invalidation pull — zéro machinerie d'événements sans consommateur.
- Déjà en place ailleurs : le cache de textures du renderer remplace sous la même clé (V3.13), `ShaderLibrary::register` remplace en avertissant.
- Futur, documenté : watcher de fichiers (décision de dépendance `notify`), debounce, re-dérivation GPU automatique à la couture, propagation aux materials/pipelines.

## Distribution — la couture

Le pipeline distribue au renderer via **`chaos_engine::assets`** — le seul module qui voit les deux mondes (le patron raw-window-handle) :

- **Le service** : `EngineContext::assets()` / `assets_mut()` — l'Asset Manager est un service du contexte (toujours présent, aucune dépendance GPU) ; le contenu déclare, importe et récupère ses ressources par lui, jamais autrement.
- **Le pont** : `texture_descriptor(label, &TextureData, format)` (le choix sRGB/linéaire appartient à l'appelant) et `textured_geometry(name, &MeshData)` — c'est **ici** que vit la limite u16 du renderer : plus de 65 536 sommets → erreur explicite ; les indices, déjà validés par la porte du pipeline, se convertissent sans risque.
- **La preuve vivante** : le sol de la démo vient de fichiers réels (`assets/textures/checker.ppm`, `assets/models/floor.glb`) — déclarés, importés, cousus, affichés. Chemins relatifs à la racine du workspace (le montage/VFS viendra plus tard).
- Le renderer n'a **jamais été modifié** pendant la phase : il consomme des descripteurs, point.

## Ce que les phases suivantes brancheront ici

- **Formats riches** : PNG (avec sa décision de dépendance), format material (`ImportedAsset` l'accueillera).
- **Scene System / ECS / Editor** : ils s'appuient sur le pipeline (référencer par `AssetId`, acquérir/relâcher, lister via le registre) — sans le modifier.
- **Hot reload effectif** : watcher + `reload(id)` + re-dérivation à la couture (comparer les versions).
