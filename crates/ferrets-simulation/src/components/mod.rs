//! ECS components for simulation entities.
//!
//! Each component covers a single concern. Not all entities carry all components —
//! optional behaviors (movement, combat, …) are expressed by component presence.

pub mod dying;
pub mod entity_info;
pub mod location;
pub mod movement;
pub mod order_queue;
