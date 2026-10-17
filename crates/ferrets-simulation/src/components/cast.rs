//! The walk a cast takes before it lands.

use bevy_ecs::prelude::*;

use ferrets_content::skills::Refund;

use crate::{components::chase::ChaseState, session::player_id::PlayerId};

/// How far a cast has got: still working toward its point, or standing
/// through what is left of itself.
///
/// A cast lands once, and which side of that it is on is remembered here
/// rather than derived from the phase, which a stat moving under it can leave
/// behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastStage {
    /// Working toward the tick the effect lands on.
    Working,
    /// Landed, and standing through the rest of the cast's own period.
    Holding,
}

/// What a running cast has paid so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paid {
    /// Nothing: it pays when it lands, or its casting has not started.
    Nothing,
    /// Its costs and cooldown, when its casting started.
    AtStart {
        /// Who paid the resources it cost.
        player: PlayerId,
        /// What it gives back if cut short before it lands.
        refund: Refund,
    },
}

/// Drives a running Cast order: what its walk toward the aim has done so far.
///
/// What is being cast and at what travels on the order itself, as every other
/// order's target does; only the walk needs remembering between ticks.
#[derive(Component, Debug)]
pub struct CastComponent {
    /// The last round of the chase toward the aim, for judging progress.
    pub last_chase: ChaseState,
    /// Ticks of casting since the caster settled into reach; zero while it
    /// is still walking there.
    pub phase: u32,
    /// Whether the effect has landed yet.
    pub stage: CastStage,
    /// What it has paid.
    pub paid: Paid,
}

impl CastComponent {
    /// A cast about to start: no chase walked yet, no tick of casting, still
    /// working toward its point, and nothing paid.
    pub fn starting() -> Self {
        Self {
            last_chase: None,
            phase: 0,
            stage: CastStage::Working,
            paid: Paid::Nothing,
        }
    }
}
