//! Deterministic simulation identifier, stable across all peers and replays.

use bevy_ecs::prelude::*;

/// A monotonically increasing identifier assigned to every simulation entity at spawn time.
///
/// Unlike Bevy's [`Entity`], [`SimulationId`] is identical on all clients because entities are
/// spawned in the same deterministic order everywhere. Use [`SimulationId`] in commands and replays;
/// use [`Entity`] for internal ECS queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SimulationId(pub u32);

/// Tracks the next [`SimulationId`] to assign.
///
/// Call [`generate`](SimulationIdGenerator::generate) before spawning each entity and attach the returned
/// [`SimulationId`] to it. The counter must be incremented in the same deterministic order on
/// every peer to keep IDs consistent across clients and replays.
#[derive(Resource, Default)]
pub struct SimulationIdGenerator(u32);

impl SimulationIdGenerator {
    pub fn generate(&mut self) -> SimulationId {
        self.0 += 1;
        SimulationId(self.0)
    }
}
