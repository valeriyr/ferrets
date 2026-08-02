//! Deterministic RTS game simulation.

pub mod buffs_store;
pub mod checksum;
pub mod command;
pub mod components;
pub mod content;
pub mod control_groups;
pub mod entity_def;
pub mod entity_index;
pub mod game_loop;
pub mod impacts;
pub mod input;
pub mod map;
pub mod map_data;
pub mod order;
pub mod player_buffs;
pub mod player_research;
pub mod player_skills;
pub mod player_stats;
pub mod requirements;
pub mod resources;
pub mod scenario;
pub mod selection;
pub mod session;
pub mod simulation_id;
pub mod skirmish;
pub mod spawn;
pub mod supply;
pub mod visibility;

/// The full build version, `major.minor.patch`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compatibility version, `major.minor`. The patch is excluded, so patch
/// releases are compatible (`1.0.0` with `1.0.1`).
pub const PROTOCOL_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
);
