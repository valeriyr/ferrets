//! How much of the map a scripted player observes.

use serde::{Deserialize, Serialize};

/// How much of the map a scripted player observes. Part of the seat — synced
/// with the slots and recorded in a replay — because it decides how the
/// player's commands resolve: a fog-limited player cannot name what fog
/// hides, an omniscient one can. The brain declares it in content; the seat
/// carries the declaration to every node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiVision {
    /// Sees only what its team's vision reveals — fog of war applies, to its
    /// view and to its commands.
    Filtered,
    /// Sees the whole map, ignoring fog — its view and its commands both
    /// reach through it.
    Omniscient,
}
