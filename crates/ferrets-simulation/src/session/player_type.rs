//! Who occupies a player slot.

use serde::{Deserialize, Serialize};

/// Who occupies a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerType {
    /// Controlled by a human.
    Human,
    /// Controlled by an AI script.
    Ai,
}
