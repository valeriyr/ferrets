//! Marker for workers sitting in a berth of the job they attend.

use bevy_ecs::prelude::*;
use ferrets_content::work::BerthStance;
use ferrets_math::{FixedU64, facing::Facing};

use crate::simulation_id::SimulationId;

/// Which way round the group's loop a circling worker walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// Through the points in the order they are declared.
    Forward,
    /// Through the points in the reverse of the order they are declared.
    Backward,
}

/// What an attached worker is doing about its berth right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sitting {
    /// At its berth, working.
    Seated {
        /// Ticks worked at this berth since sitting down.
        dwelt: u32,
    },
    /// On the way from the berth it left to the berth it holds — round the
    /// group's loop or straight across, as its stance says.
    Travelling {
        /// The berth left behind.
        from: usize,
        /// Cells travelled since leaving it.
        along: FixedU64,
        /// Which way round the loop a circling worker walks; a roaming one
        /// crosses straight and does not read it.
        way: Way,
    },
    /// Circling its berth without leaving it.
    Orbiting {
        /// Where on the circle it is, as a bearing from the berth.
        phase: Facing,
    },
}

/// Marks a worker sitting in a berth of the job it attends — a builder on its
/// site, a carrier in its tree.
///
/// An attached entity is on the map: it can be seen, selected and targeted,
/// and it keeps its sight. It holds no cells on the navigation grid, so
/// nothing is blocked by it and nothing pushes it.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct AttachedComponent {
    /// The job whose berth the worker holds.
    pub job: SimulationId,
    /// The berth group of the job the worker sits in.
    pub berths: String,
    /// The index of the berth the worker holds in its group — the one it sits
    /// at, or is on its way to.
    pub seat: usize,
    /// How the worker sits in its berth over time.
    pub stance: BerthStance,
    /// What the worker is doing about its berth right now.
    pub sitting: Sitting,
    /// How many berths the worker has moved between on this job.
    pub hops: u32,
}
