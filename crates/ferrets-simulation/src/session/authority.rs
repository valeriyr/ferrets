//! Who steers a running session.

use serde::{Deserialize, Serialize};

use crate::session::ai_hosting::AiHosting;

/// The in-game decision authority: who resolves session-level questions that
/// every node must answer identically — dropping a stalled player, pausing at
/// an agreed tick.
///
/// Choices that only make sense with a host live inside
/// [`Host`](Self::Host), so a configuration that needs a host without having
/// one cannot be expressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authority {
    /// The session host decides and announces; every other node applies. The
    /// session cannot outlive the host.
    Host {
        /// How AI player input is computed — a host may compute it for
        /// everyone.
        ai_hosting: AiHosting,
    },
    /// No node is special once the session starts: decisions commit by
    /// consensus of the live players, and the session survives any single
    /// node — including the one that hosted the lobby. AI input is
    /// necessarily [`Replicated`](AiHosting::Replicated).
    Peers,
}

impl Authority {
    /// How AI player input is computed under this authority.
    pub fn ai_hosting(&self) -> AiHosting {
        match self {
            Self::Host { ai_hosting } => *ai_hosting,
            Self::Peers => AiHosting::Replicated,
        }
    }
}
