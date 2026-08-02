//! ECS components for simulation entities.
//!
//! Each component covers a single concern. Not all entities carry all components —
//! optional behaviors (movement, combat, …) are expressed by component presence.

pub mod attack;
pub mod attack_move;
pub mod build;
pub mod dying;
pub mod energy;
pub mod entity_buffs;
pub mod entity_info;
pub mod entity_skills;
pub mod entity_stats;
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
pub mod repair;
pub mod research;
pub mod resource;
pub mod stance;
pub mod tags;
pub mod train;
