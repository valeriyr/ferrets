//! Per-tick simulation logic: command dispatch and order execution.

pub mod acquire;
pub mod attack;
pub mod attack_move;
pub mod auto_engage;
pub mod build;
mod chase;
mod crew;
pub mod damage;
pub mod die;
pub mod executor;
pub mod flee;
pub mod follow;
pub mod game_result;
pub mod guard;
pub mod harvest;
pub mod impacts;
pub mod movement;
pub mod orders;
pub mod patrol;
pub mod pending_reveal;
pub mod repair;
pub mod stats;
pub mod tick_counter;
pub mod train;
pub mod visibility;
mod work;
