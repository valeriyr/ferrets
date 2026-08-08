//! One simulated tick's recorded input.

use ferrets_simulation::{command::PlayerCommand, session::player_slot::PlayerId};
use serde::{Deserialize, Serialize};

/// The realized input for a single tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickRecord {
    /// The tick this input was executed at.
    pub tick: u32,
    /// Each player that issued commands, paired with them, in ascending player
    /// order. Idle players are omitted.
    pub inputs: Vec<(PlayerId, Vec<PlayerCommand>)>,
    /// Players whose drop took effect at this tick — they contribute no input
    /// from here on. Playback re-applies these so a dropped player is excluded
    /// from the game exactly as it was live, rather than replayed as idle but
    /// present. Empty on a tick where nobody dropped.
    pub dropped: Vec<PlayerId>,
    /// State checksum entering this tick, sampled at the checksum interval and
    /// `None` otherwise. Playback recomputes it to verify determinism.
    pub checksum: Option<u64>,
}
