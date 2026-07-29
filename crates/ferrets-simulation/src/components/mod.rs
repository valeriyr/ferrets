//! ECS components for simulation entities.
//!
//! Each component covers a single concern. Not all entities carry all components —
//! optional behaviors (movement, combat, …) are expressed by component presence.

pub mod attack;
pub mod attack_move;
pub mod buffs;
pub mod build;
pub mod dying;
pub mod energy;
pub mod entity_info;
pub mod follow;
pub mod guard;
pub mod health;
pub mod hidden;
pub mod location;
pub mod movement;
pub mod order_queue;
pub mod owner;
pub mod patrol;
pub mod pending_reveal;
pub mod rally;
pub mod resource;
pub mod skills;
pub mod stance;
pub mod stats;
pub mod tags;
pub mod train;
