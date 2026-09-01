//! When a session ends on its own.

use serde::{Deserialize, Serialize};

use crate::session::elimination_scope::EliminationScope;

/// When a session ends on its own.
///
/// Each game mode's ending rule is one variant carrying whatever configuration
/// the rule needs, so the session, the lobby agreement, and a recording all
/// name a mode the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishPolicy {
    /// End the game once the players still holding a building are all on one
    /// side. The default for an ordinary match.
    LastStanding {
        /// Whose buildings keep a player in the game.
        elimination: EliminationScope,
    },
    /// Never end the game automatically — it runs until stopped explicitly.
    /// Suited to sandboxes and tests, where a lone or unpopulated player slot
    /// must not be read as a win.
    Endless,
    /// An installed scenario runtime decides when the game ends and how; the
    /// built-in last-standing check stands aside. Which scenario is not the
    /// policy's business — a scenario is a whole game definition, and the
    /// session only records that its script has the verdict.
    Scripted,
}
