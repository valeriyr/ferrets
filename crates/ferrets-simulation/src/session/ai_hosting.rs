//! How AI player input is computed in a session.

use serde::{Deserialize, Serialize};

/// How AI player input is computed in a session.
///
/// The choice only moves where AI commands are produced; either way they enter
/// the committed-input store like any other player's and are recorded in
/// replays as realized input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AiHosting {
    /// Every node computes every AI locally and commits identical frames; AI
    /// frames are never relayed. Requires strictly deterministic AI scripts.
    #[default]
    Replicated,
    /// The session host computes all AIs. Each AI player keeps its own
    /// identity and frames; only their origin is the host node, so the other
    /// nodes receive them like any remote player's input. AI scripts need not
    /// be deterministic, but the AIs live and die with the host node.
    HostOnly,
}
