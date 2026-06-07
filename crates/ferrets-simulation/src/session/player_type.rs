//! Who occupies a player slot.

/// Who occupies a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerType {
    /// Controlled by a human.
    Human,
    /// Controlled by an AI script.
    Ai,
    /// Commands sourced from a replay file.
    Replay,
}
