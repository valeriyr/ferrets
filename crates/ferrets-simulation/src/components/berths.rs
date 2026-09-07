//! Runtime occupancy of a job's berths.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::simulation_id::SimulationId;

/// Who sits in each berth of a job, by berth group.
#[derive(Component, Debug, Default, Clone, PartialEq, Eq)]
pub struct BerthsComponent {
    /// The occupant of every berth of every group sat in so far.
    pub seats: BTreeMap<String, Vec<Option<SimulationId>>>,
}
