use std::fmt;

use crate::asset::fnv1a_64;

/// Identité stable et unique d'une scène, dérivée de son nom logique —
/// jamais d'un chemin de fichier. Le précédent exact d'`AssetId` : même
/// nom → même identité, déterministe à travers les sessions, les machines
/// et le réseau — sérialisable dans les sauvegardes, transmissible par un
/// serveur, stable pour le contenu distribué. L'identité est une valeur de
/// première classe ; le nom reste une métadonnée de la scène.
///
/// **Pourquoi chaos_core** : le graphe de dépendances — `chaos_network`
/// (réplication) et `chaos_api` (surface de modding) ne voient que le
/// cœur et manipuleront des identités de scènes. Le vocabulaire vit ici ;
/// le modèle de scène vit dans `chaos_scene`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SceneId(u64);

impl SceneId {
    /// Dérive l'identité du nom logique — FNV-1a 64 bits partagé avec
    /// `AssetId`, VERROUILLÉ par vecteurs de référence en test.
    pub fn from_name(name: &str) -> Self {
        Self(fnv1a_64(name))
    }
}

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene:{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_yields_the_same_id() {
        assert_eq!(
            SceneId::from_name("maps/spawn"),
            SceneId::from_name("maps/spawn")
        );
    }

    #[test]
    fn distinct_names_yield_distinct_ids() {
        let names = ["maps/spawn", "maps/spawn2", "maps/city", "a", "b", ""];
        for (i, first) in names.iter().enumerate() {
            for second in &names[i + 1..] {
                assert_ne!(
                    SceneId::from_name(first),
                    SceneId::from_name(second),
                    "collision entre '{first}' et '{second}'"
                );
            }
        }
    }

    #[test]
    fn the_algorithm_is_locked_by_reference_vectors() {
        assert_eq!(SceneId::from_name(""), SceneId(0xcbf2_9ce4_8422_2325));
        assert_eq!(SceneId::from_name("a"), SceneId(0xaf63_dc4c_8601_ec8c));
        assert_eq!(SceneId::from_name("foobar"), SceneId(0x8594_4171_f739_67e8));
    }

    #[test]
    fn display_is_stable_hex() {
        assert_eq!(SceneId::from_name("").to_string(), "scene:cbf29ce484222325");
    }
}
