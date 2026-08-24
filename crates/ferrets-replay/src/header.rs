//! The replay header: everything needed to set up a game before its recorded
//! input is replayed.

use ferrets_geometry::projection::Projection;
use ferrets_simulation::{movement_model::MovementModel, skirmish::Skirmish};
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
    /// The game the recording is of.
    pub game: RecordedGame,
    /// How bodies occupied space in the recorded game.
    pub movement_model: MovementModel,
    /// The distance metric the recorded game was played under.
    pub projection: Projection,
}

/// How the recorded game was defined.
///
/// Named content (a scenario, a skirmish's map) is recorded by name: like the
/// content and the simulation logic, it must be known identically to whoever
/// replays the recording — the embedded checksums expose a drifted one as a
/// desync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordedGame {
    /// A lobby-made game, spelled out.
    Skirmish(Skirmish),
    /// A scenario game: the named scenario defines everything else.
    Scenario(String),
}

impl ReplayHeader {
    /// Builds a header stamped with the current format and engine version.
    ///
    /// A replay is the same for every participant, and a viewer is a spectator.
    /// Which slot to follow is the viewer's choice.
    ///
    /// The movement model and the projection are recorded because they shape the
    /// simulation without being derivable from the game's name: a recording
    /// rebuilt under the other model or metric is a different game, and shows up
    /// as a checksum mismatch rather than a load failure.
    pub fn new(game: RecordedGame, movement_model: MovementModel, projection: Projection) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            engine_version: ferrets_simulation::VERSION.to_string(),
            game,
            movement_model,
            projection,
        }
    }
}
