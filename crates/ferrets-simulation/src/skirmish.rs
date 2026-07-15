//! An ordinary game described as data: who plays, where, and how it ends.

use serde::{Deserialize, Serialize};

use crate::session::finish_policy::FinishPolicy;
use crate::session::player_slot::PlayerSlot;

/// A lobby-made game — the counterpart of a scenario. Where a scenario is an
/// authored package resolved by name, a skirmish has no identity of its own:
/// it *is* its configuration, spelled out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skirmish {
    /// The player slots the game runs with.
    pub slots: Vec<PlayerSlot>,
    /// The map the game is played on, by name.
    pub map: String,
    /// When the game ends on its own.
    pub finish_policy: FinishPolicy,
}
