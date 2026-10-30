//! Deterministic RTS game simulation.

pub mod annex;
pub mod berths;
pub mod beside;
pub mod bore_id;
pub mod brood;
pub mod buffs_store;
pub mod checksum;
pub mod command;
pub mod components;
pub mod control_groups;
pub mod cooldown;
pub mod entity_def;
pub mod entity_index;
pub mod events;
pub mod fields;
pub mod game_loop;
pub mod ground;
pub mod impacts;
pub mod input;
pub mod landing;
pub mod links;
pub mod make_way;
pub mod map;
pub mod map_data;
pub mod movement_model;
pub mod networks;
pub mod occupancy;
pub mod order;
mod overbuild;
pub mod player_buffs;
pub mod player_research;
pub mod player_skills;
pub mod player_stats;
pub mod rally;
pub mod reach;
mod regions;
pub mod requirements;
pub mod resources;
pub mod ruleset;
pub mod scenario;
pub mod selection;
pub mod session;
mod shadows;
pub mod simulation_id;
pub mod skirmish;
pub mod spawn;
mod stat_store;
pub mod statistics;
pub mod supply;
pub mod tunnels;
pub mod visibility;
pub mod watches;

/// The full build version, `major.minor.patch`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compatibility version, `major.minor`. The patch is excluded, so patch
/// releases are compatible (`1.0.0` with `1.0.1`).
pub const PROTOCOL_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
);
