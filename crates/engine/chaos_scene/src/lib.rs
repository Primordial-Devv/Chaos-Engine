//! Scene System de Chaos Engine : organiser et conserver le monde,
//! AU-DESSUS de l'ECS — jamais un second ECS. L'ECS fait vivre les
//! entités ; la scène décrit une portion de monde, l'organise et
//! conservera son état. Le renderer consommera, l'Asset Pipeline
//! fournira — la scène ne connaît ni l'un ni l'autre.
//! Architecture détaillée : `docs/scene/overview.md`.

pub mod hierarchy;
pub mod manager;
pub mod member;
pub mod mesh_ref;
pub mod prefab;
pub mod propagation;
pub mod scene;
pub mod serialization;

pub use hierarchy::ChildOf;
pub use manager::SceneManager;
pub use member::SceneMember;
pub use mesh_ref::MeshRef;
pub use prefab::Prefab;
pub use propagation::TransformPropagation;
pub use scene::{Scene, SceneState};
pub use serialization::{EntityData, FORMAT_VERSION, SceneData};
