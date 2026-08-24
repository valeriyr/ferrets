//! The cadence the game plays at: the tick rate it installs, and the speeds a
//! player may choose between.
//!
//! Only the *choice* lives here. Holding to it is the engine's business — it
//! scales this cadence by the chosen speed and throttles it when a tick, or a
//! peer, cannot keep up (see `ferrets_bevy_plugin::tick`).

use ferrets_math::FixedU64;
use ferrets_simulation::session::game_speed::GameSpeed;

/// The cadence one tick is computed at when nothing scales it: 20 ticks per
/// second. The engine never states a rate of its own — it scales this one by the
/// session's speed.
pub const NOMINAL_TICK_HZ: f64 = 20.0;
/// A speed the game offers. The engine takes any positive factor; these are the
/// steps a player may pick from, and the top two are fast-forward — offered only
/// where nobody is competing (a replay, or a game off the network).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpeedStep {
    Slowest,
    Slow,
    #[default]
    Normal,
    Fast,
    Faster,
    Fastest,
}

impl SpeedStep {
    /// Every rung, slowest first.
    pub const LADDER: [Self; 6] = [
        SpeedStep::Slowest,
        SpeedStep::Slow,
        SpeedStep::Normal,
        SpeedStep::Fast,
        SpeedStep::Faster,
        SpeedStep::Fastest,
    ];

    /// The factor this step scales the nominal cadence by. Every step lands on a
    /// whole number of ticks per second against [`NOMINAL_TICK_HZ`].
    pub fn factor(self) -> FixedU64 {
        let (numerator, denominator) = match self {
            SpeedStep::Slowest => (1, 4),
            SpeedStep::Slow => (1, 2),
            SpeedStep::Normal => (1, 1),
            SpeedStep::Fast => (2, 1),
            SpeedStep::Faster => (4, 1),
            SpeedStep::Fastest => (8, 1),
        };
        FixedU64::from_num(numerator) / FixedU64::from_num(denominator)
    }

    /// The session speed this step asks for.
    pub fn speed(self) -> GameSpeed {
        GameSpeed::new(self.factor())
    }

    /// Whether this step is a fast-forward one, past the pace a game is meant to
    /// be played at.
    pub fn fast_forward(self) -> bool {
        match self {
            SpeedStep::Faster | SpeedStep::Fastest => true,
            SpeedStep::Slowest | SpeedStep::Slow | SpeedStep::Normal | SpeedStep::Fast => false,
        }
    }

    /// The next step up, or this one at the top of the ladder.
    pub fn faster(self) -> Self {
        match self {
            SpeedStep::Slowest => SpeedStep::Slow,
            SpeedStep::Slow => SpeedStep::Normal,
            SpeedStep::Normal => SpeedStep::Fast,
            SpeedStep::Fast => SpeedStep::Faster,
            SpeedStep::Faster | SpeedStep::Fastest => SpeedStep::Fastest,
        }
    }

    /// The next step down, or this one at the bottom of the ladder.
    pub fn slower(self) -> Self {
        match self {
            SpeedStep::Slowest | SpeedStep::Slow => SpeedStep::Slowest,
            SpeedStep::Normal => SpeedStep::Slow,
            SpeedStep::Fast => SpeedStep::Normal,
            SpeedStep::Faster => SpeedStep::Fast,
            SpeedStep::Fastest => SpeedStep::Faster,
        }
    }

    /// How this step reads on screen.
    pub fn label(self) -> &'static str {
        match self {
            SpeedStep::Slowest => "0.25x",
            SpeedStep::Slow => "0.5x",
            SpeedStep::Normal => "1x",
            SpeedStep::Fast => "2x",
            SpeedStep::Faster => "4x",
            SpeedStep::Fastest => "8x",
        }
    }

    /// The rung `speed` sits on.
    ///
    /// The session is the one owner of the game's speed — in a network game a
    /// peer's change lands there without this node pressing anything — so the
    /// rung is derived from it wherever the keys step or the readout names it,
    /// never remembered on the side where it could drift. Every speed the
    /// session can hold came from this ladder (a rung the local player picked,
    /// or one a peer picked), so the lookup always finds its step; the fallback
    /// is for a factor no honest node can produce.
    pub fn of(speed: GameSpeed) -> Self {
        Self::LADDER
            .into_iter()
            .find(|step| step.speed() == speed)
            .unwrap_or(SpeedStep::Normal)
    }
}
