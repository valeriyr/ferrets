//! When a node turns a stalled player into its drop decision.

use serde::{Deserialize, Serialize};

/// How a stall becomes a drop decision on the nodes whose decision counts —
/// the deciding host, or every node under peer authority. Agreed per session,
/// like the rest of the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropPolicy {
    /// Decide as soon as the grace window elapses.
    Automatic,
    /// Never decide on a timer: the stall is surfaced to the game and the
    /// decision waits for its explicit approval (e.g. a wait-for-player
    /// dialog).
    Manual,
}
