//! Content-declared time: how long something takes, as a tick count or as a
//! stat read from the entity it happens to.

use crate::entity_stats::EntityStatId;

/// How long something content declares takes, or how often it recurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// A fixed number of ticks. Zero completes in the same tick it starts,
    /// or recurs every tick.
    Constant(u32),
    /// Read from the entity's effective stats each tick, so the modifier
    /// pipeline can move it while the work is under way.
    Stat(EntityStatId),
}
