//! When a session ends on its own.

use serde::{Deserialize, Serialize};

/// When a session ends on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishPolicy {
    /// End the game once only one player (or none) still has entities. The
    /// default for an ordinary match.
    LastStanding,
    /// Never end the game automatically — it runs until stopped explicitly.
    /// Suited to sandboxes and tests, where a lone or unpopulated player slot
    /// must not be read as a win.
    Endless,
}
