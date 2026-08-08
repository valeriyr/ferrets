//! In-flight repair state for simulation entities.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use crate::components::chase::ChaseState;
use crate::simulation_id::SimulationId;

/// The crew mending an entity, present exactly while somebody is on the job.
#[derive(Component, Debug, Default)]
pub struct UnderRepairComponent {
    /// The workers mending it right now, from the moment each takes the job on —
    /// including those still walking to it.
    pub repairers: BTreeSet<SimulationId>,
}

/// Per-entity in-flight repair state.
#[derive(Component, Debug)]
pub struct RepairComponent {
    /// What is being mended.
    pub target: SimulationId,
    /// Set once a worker that mends from inside its job has stepped into this one —
    /// the only case with no walk left to run, since such a worker holds no cell and
    /// cannot be asked to move. One that mends from the open closes on its target
    /// every tick instead, because a patient can walk away from the hands mending it.
    pub inside_job: bool,
    /// `(own position, target position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: ChaseState,
    /// Consecutive ticks spent unable to pay for the work.
    pub stalled: u32,
    /// Fractional cost carried between ticks, by resource kind. Work lands
    /// continuously while stockpiles are whole numbers, so the remainder waits here
    /// rather than being rounded away or charged twice.
    pub owed: BTreeMap<String, FixedU64>,
}

impl RepairComponent {
    /// Creates repair state for a worker starting on `target`.
    pub fn new(target: SimulationId) -> Self {
        Self {
            target,
            inside_job: false,
            last_chase: None,
            stalled: 0,
            owed: BTreeMap::new(),
        }
    }
}
