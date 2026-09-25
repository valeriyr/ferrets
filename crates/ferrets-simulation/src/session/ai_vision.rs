//! How much of the map a scripted player observes.

use serde::{Deserialize, Serialize};

/// How much of the map a scripted player observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiVision {
    /// Sees only what its team's vision reveals — fog of war applies, to its
    /// view and to its commands.
    Filtered,
    /// Sees the whole map, ignoring fog — its view and its commands both
    /// reach through it.
    Omniscient,
}
