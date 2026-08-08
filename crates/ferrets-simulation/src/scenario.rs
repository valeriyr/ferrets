//! A mission described as data: the cast it is played by, the map it is
//! played on, and the script that judges its objectives and outcome.
//!
//! A [`Scenario`] is plain serializable data — it carries its script as
//! source and interprets nothing itself, so the same value can be stored in
//! a file, embedded in a recording, or sent over a wire. Everything placed on
//! the opening scene, owned or neutral, belongs to the embedded
//! [`MapData`]; the scenario adds only what is mission-specific.

use serde::{Deserialize, Serialize};

use crate::{
    map_data::MapData,
    resources::StartingStock,
    session::{
        player_slot::{PlayerId, TeamId},
        player_type::PlayerType,
    },
};

/// One authored cast assignment: who occupies one of the map's player seats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioPlayer {
    /// The map player seat the assignment fills.
    pub seat: PlayerId,
    /// Who controls the seat.
    pub player_type: PlayerType,
    /// The race played, by registered race name.
    pub race: Option<String>,
    /// The team played on, or `None` for no team.
    pub team: Option<TeamId>,
}

/// A mission: the cast that plays it, the map it opens on, and the script
/// that judges it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// A unique scenario identifier.
    pub name: String,
    /// The authored cast, one entry per occupied player seat. Seats the cast
    /// leaves out stay free; the map's environment seats need no entry.
    pub players: Vec<ScenarioPlayer>,
    /// The seat whose progress the script judges.
    pub judged_player: PlayerId,
    /// The map the mission is played on, placements included.
    pub map: MapData,
    /// What each player's stockpile starts with.
    pub stockpile: Vec<StartingStock>,
    /// The scenario script, carried as source.
    pub script: String,
}
