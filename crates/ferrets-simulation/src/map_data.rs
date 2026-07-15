//! A map described as serializable data, from which a live map is built.

use ferrets_pathfinder::{astar::Projection, nav_grid::LayerId};
use serde::{Deserialize, Serialize};

use crate::session::player_slot::PlayerId;

/// One entity a map opens with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The entity type to spawn, by content name.
    pub type_name: String,
    /// The cell to spawn it on.
    pub cell: (u32, u32),
    /// The owning slot, or `None` for a neutral entity.
    pub owner: Option<PlayerId>,
    /// Overrides the spawned resource source's starting amount.
    pub amount: Option<u32>,
}

/// A map described as data: its grid, its navigation layers, where each player
/// starts, and what stands on it before the first tick.
///
/// Everything player-agnostic about a game's opening scene belongs here —
/// neutral resources are placements with no owner, and owner-tagged placements
/// key their owner by slot id. What a slot's player *does* get at start (a
/// base, a stockpile) is the game's rule, not the map's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapData {
    /// A unique map identifier.
    pub name: String,
    /// Movement cost model and range metric used across the entire map.
    pub projection: Projection,
    /// Playable width in cells.
    pub width: u32,
    /// Playable height in cells.
    pub height: u32,
    /// The navigation layers the map registers.
    pub layers: Vec<LayerId>,
    /// Player start cells, ordered by slot id.
    pub start_points: Vec<(u32, u32)>,
    /// The entities the map opens with, built in declared order.
    pub placements: Vec<Placement>,
}
