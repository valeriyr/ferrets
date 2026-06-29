//! Gameplay traffic: the lockstep frames and desync checksums.
//!
//! These ride the gameplay channel. The lobby/control protocol is a separate
//! concern ([`ControlMessage`](super::control::ControlMessage)).

use ferrets_simulation::input::PlayerFrame;
use serde::{Deserialize, Serialize};

/// A message on the gameplay channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameplayMessage {
    /// A batch of player frames — the lockstep payload. Carries the local
    /// player's scheduled frames plus, on a relaying node, frames it holds for
    /// other players, across a small window of ticks for redundancy. Each frame
    /// carries its originator's `player`, preserved across relay hops.
    Frames(Vec<PlayerFrame>),
    /// A state checksum for `tick`, compared across peers to detect desyncs.
    Sync { tick: u32, hash: u64 },
}
