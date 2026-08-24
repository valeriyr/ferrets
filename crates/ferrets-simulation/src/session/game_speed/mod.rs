//! How fast the session runs in wall-clock terms.

pub mod error;

use ferrets_math::FixedU64;
use serde::{Deserialize, Serialize};

use crate::session::game_speed::error::GameSpeedError;

/// A multiplier on the game's nominal tick cadence: `2` runs twice as many ticks
/// per real second, `1/2` half as many.
///
/// Only the *duration* of a tick changes with it, never the number of ticks, so
/// no simulated outcome depends on it and it stays out of the checksum. The
/// nominal cadence itself is the game's to choose, and so is the set of factors
/// a player may pick from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "FixedU64")]
pub struct GameSpeed(FixedU64);

impl GameSpeed {
    /// The nominal cadence, unscaled.
    pub const NORMAL: Self = Self(FixedU64::ONE);

    /// Builds a speed from its factor.
    ///
    /// Panics if `factor` is zero — a frozen tick loop is
    /// [`set_paused`](super::GameSession::set_paused), not a speed. A factor
    /// arriving from outside the program goes through deserialization instead,
    /// where a zero is refused as an error rather than a panic.
    pub fn new(factor: FixedU64) -> Self {
        Self::try_from(factor).unwrap_or_else(|error| panic!("{error}"))
    }

    /// The factor to scale the nominal cadence by.
    pub fn factor(self) -> FixedU64 {
        self.0
    }
}

impl Default for GameSpeed {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl TryFrom<FixedU64> for GameSpeed {
    type Error = GameSpeedError;

    fn try_from(factor: FixedU64) -> Result<Self, Self::Error> {
        if factor == FixedU64::ZERO {
            return Err(GameSpeedError::ZeroSpeedFactor);
        }
        Ok(Self(factor))
    }
}
