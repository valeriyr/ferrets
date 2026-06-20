//! Per-tick simulation logic: command dispatch and order execution.

pub mod attack;
pub mod build;
mod chase;
pub mod die;
pub mod executor;
pub mod follow;
pub mod harvest;
pub mod movement;
pub mod orders;
pub mod pending_reveal;
pub mod tick_counter;
pub mod train;
