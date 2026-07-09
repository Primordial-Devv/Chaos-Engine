//! Asset Pipeline de Chaos Engine : le producteur des ressources.
//! Le registre catalogue (identité, provenance, type, état) ; les
//! sous-phases suivantes chargeront et distribueront. Le renderer et les
//! autres sous-systèmes consomment — jamais l'inverse.
//! Architecture détaillée : `docs/assets/overview.md`.

pub mod import;
pub mod importers;
pub mod manager;
pub mod registry;

pub use import::{AssetImporter, ImportedAsset, MeshData, TextureData};
pub use importers::{GltfImporter, PpmImporter, WgslImporter};
pub use manager::AssetManager;
pub use registry::{AssetEntry, AssetKind, AssetRegistry, AssetSource, AssetState};
