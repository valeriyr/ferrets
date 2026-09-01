//! Who occupies a player slot.

use serde::{Deserialize, Serialize};

use crate::session::ai_vision::AiVision;

/// Who occupies a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerType {
    /// Controlled by a human.
    Human,
    /// Controlled by an AI script.
    Ai {
        /// How much of the map the script observes.
        vision: AiVision,
    },
}
