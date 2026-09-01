//! What a node does when its local player is defeated while the match goes on.

use serde::{Deserialize, Serialize};

/// What a node does when its local player is defeated — its whole side
/// eliminated under [`FinishPolicy::LastStanding`](crate::session::finish_policy::FinishPolicy)
/// — while unallied survivors fight on.
///
/// A local presentation choice, not part of the session agreement: the defeat
/// it answers is itself derived per node, every node keeps deriving the same
/// eliminations whatever conduct it runs under, and so two peers may choose
/// differently without desync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefeatConduct {
    /// Finish the local session with [`Defeat`](crate::session::GameResult::Defeat)
    /// — the node stops at the frozen losing frame.
    Conclude,
    /// Keep the node simulating so the player watches the match play out; the
    /// shared result arrives when one side is left standing.
    Spectate,
}
