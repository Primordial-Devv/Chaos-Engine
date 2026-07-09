use std::fmt;

/// Identité stable et unique d'une ressource du moteur, dérivée de son nom
/// logique — jamais d'un chemin de fichier. Déterministe à travers les
/// sessions et les machines : même nom → même identité, donc sérialisable
/// dans les scènes, transmissible sur le réseau et stable pour le modding.
///
/// Le nom logique est virtuel (convention : minuscules, séparateur `/`,
/// sans extension — ex. `textures/brick`) ; c'est l'Asset Pipeline qui
/// mappe les fichiers réels vers ces noms, jamais l'inverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(u64);

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl AssetId {
    /// Dérive l'identité du nom logique — FNV-1a 64 bits, octet-exact,
    /// VERROUILLÉ par vecteurs de référence en test : changer cet
    /// algorithme invaliderait toute référence d'asset sérialisée.
    pub fn from_name(name: &str) -> Self {
        let mut hash = FNV_OFFSET_BASIS;
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        Self(hash)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "asset:{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_yields_the_same_id() {
        assert_eq!(
            AssetId::from_name("textures/brick"),
            AssetId::from_name("textures/brick")
        );
    }

    #[test]
    fn distinct_names_yield_distinct_ids() {
        let names = [
            "textures/brick",
            "textures/bricks",
            "textures/brick2",
            "models/brick",
            "a",
            "b",
            "",
        ];
        for (i, first) in names.iter().enumerate() {
            for second in &names[i + 1..] {
                assert_ne!(
                    AssetId::from_name(first),
                    AssetId::from_name(second),
                    "collision entre '{first}' et '{second}'"
                );
            }
        }
    }

    #[test]
    fn the_algorithm_is_locked_by_reference_vectors() {
        assert_eq!(AssetId::from_name(""), AssetId(0xcbf2_9ce4_8422_2325));
        assert_eq!(AssetId::from_name("a"), AssetId(0xaf63_dc4c_8601_ec8c));
        assert_eq!(AssetId::from_name("foobar"), AssetId(0x85944171f73967e8));
    }

    #[test]
    fn hashing_is_byte_exact() {
        assert_ne!(
            AssetId::from_name("Textures/Brick"),
            AssetId::from_name("textures/brick")
        );
        assert_ne!(
            AssetId::from_name("textures/brick"),
            AssetId::from_name("textures\\brick")
        );
    }

    #[test]
    fn display_is_stable_hex() {
        assert_eq!(AssetId::from_name("").to_string(), "asset:cbf29ce484222325");
    }
}
