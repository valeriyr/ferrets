//! The walk a cast takes before it lands.

use bevy_ecs::prelude::*;

use crate::components::chase::ChaseState;

/// How far a cast has got: still working toward its point, or standing
/// through what is left of itself.
///
/// A cast lands once, and which side of that it is on is remembered here
/// rather than derived from the phase, which a stat moving under it can leave
/// behind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CastStage {
    /// Working toward the tick the effect lands on.
    #[default]
    Working,
    /// Landed, and standing through the rest of the cast's own period.
    Holding,
}

/// Drives a running Cast order: what its walk toward the aim has done so far.
///
/// What is being cast and at what travels on the order itself, as every other
/// order's target does; only the walk needs remembering between ticks.
#[derive(Component, Debug, Default)]
pub struct CastComponent {
    /// The last round of the chase toward the aim, for judging progress.
    pub last_chase: ChaseState,
    /// Ticks worked at the cast since the caster settled into reach. It starts
    /// over whenever the caster has to walk again.
    pub phase: u32,
    /// Whether the effect has landed yet.
    pub stage: CastStage,
}
