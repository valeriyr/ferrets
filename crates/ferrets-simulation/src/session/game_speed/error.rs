//! The game speed error type.

/// Something went wrong with the game speed.
#[derive(Debug, thiserror::Error)]
pub enum GameSpeedError {
    #[error("a game speed factor cannot be zero")]
    ZeroSpeedFactor,
}
