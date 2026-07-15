//! A mission described as data: the session it runs with, the map it is
//! played on, and the script that judges its objectives and outcome.
//!
//! A [`Scenario`] is plain serializable data — it carries its script as
//! source and interprets nothing itself, so the same value can be stored in
//! a file, embedded in a recording, or sent over a wire. Everything placed on
//! the opening scene, owned or neutral, belongs to the embedded
//! [`MapData`]; the scenario adds only what is mission-specific.

use serde::{Deserialize, Serialize};

use crate::map_data::MapData;
use crate::resources::StartingStock;
use crate::session::player_slot::{PlayerId, PlayerSlot};

/// A mission: the session it runs with, the map it opens on, and the script
/// that judges it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// A unique scenario identifier.
    pub name: String,
    /// The player slots the mission runs with.
    pub slots: Vec<PlayerSlot>,
    /// The slot whose progress the script judges.
    pub judged_player: PlayerId,
    /// The map the mission is played on, placements included.
    pub map: MapData,
    /// What each player's stockpile starts with.
    pub stockpile: Vec<StartingStock>,
    /// The scenario script, carried as source.
    pub script: String,
}
