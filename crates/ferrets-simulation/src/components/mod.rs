//! ECS components for simulation entities.
//!
//! Each component covers a single concern. Not all entities carry all components —
//! optional behaviors (movement, combat, …) are expressed by component presence.

pub mod attack;
pub mod build;
pub mod dying;
pub mod entity_info;
pub mod follow;
pub mod health;
pub mod hidden;
pub mod location;
pub mod movement;
pub mod order_queue;
pub mod owner;
pub mod pending_reveal;
pub mod rally;
pub mod resource;
pub mod tags;
pub mod train;
