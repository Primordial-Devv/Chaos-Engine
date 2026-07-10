//! ECS de Chaos Engine : le cœur logique du moteur. Le monde = des entités
//! (identités pures, `chaos_core::Entity`), des composants (leurs données)
//! et des systèmes (leur comportement) — cette crate le fait vivre. Le
//! renderer consomme ce que le monde produit, l'Asset Pipeline fournit ses
//! ressources ; l'ECS représente.
//! Architecture détaillée : `docs/ecs/overview.md`.

pub mod command;
pub mod component;
pub mod entities;
pub mod message;
mod query;
pub mod resource;
pub mod schedule;
pub mod storage;
pub mod system;
pub mod world;

pub use command::Commands;
pub use component::Component;
pub use entities::Entities;
pub use message::{Message, Messages};
pub use resource::{Resource, Resources};
pub use schedule::Schedule;
pub use storage::ComponentStorage;
pub use system::{System, Systems};
pub use world::World;
