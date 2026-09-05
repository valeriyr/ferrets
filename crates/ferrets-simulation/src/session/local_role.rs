//! The role a node holds in the session it runs.

use serde::{Deserialize, Serialize};

use crate::session::player_id::PlayerId;

/// The role a node holds in the session: it fields a player, or it only
/// watches. A watching node carries no identity here — the simulation is
/// bit-identical without its watchers, so there is nothing session-level to
/// name one by; who actually watches is the lobby's and the network layer's
/// knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalRole {
    /// Fields the player in this slot.
    Player(PlayerId),
    /// Watches: controls nothing, and the game knows nothing of it.
    Observer,
}

impl LocalRole {
    /// The slot this role fields, or `None` for a watcher.
    pub fn player(self) -> Option<PlayerId> {
        match self {
            LocalRole::Player(player) => Some(player),
            LocalRole::Observer => None,
        }
    }
}
