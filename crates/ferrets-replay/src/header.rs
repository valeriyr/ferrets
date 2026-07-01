//! The replay header: everything needed to set up a game before its recorded
//! input is replayed.

use ferrets_simulation::session::{FinishPolicy, player_slot::PlayerSlot};
use serde::{Deserialize, Serialize};

/// The replay file format this build writes and reads. Bumped whenever the
/// on-disk layout or the recorded state-checksum changes.
pub const FORMAT_VERSION: u32 = 1;

/// The setup a replayed game is rebuilt from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayHeader {
    /// The format the replay was written in.
    pub format_version: u32,
    /// The engine version that recorded the replay.
    pub engine_version: String,
    /// The player slots the game ran with.
    pub slots: Vec<PlayerSlot>,
    /// When the recorded game ended on its own.
    pub finish_policy: FinishPolicy,
}

impl ReplayHeader {
    /// Builds a header stamped with the current format and engine version.
    ///
    /// A replay is the same for every participant, and a viewer is a spectator.
    /// Which slot to follow is the viewer's choice.
    pub fn new(slots: Vec<PlayerSlot>, finish_policy: FinishPolicy) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            engine_version: ferrets_simulation::VERSION.to_string(),
            slots,
            finish_policy,
        }
    }
}
