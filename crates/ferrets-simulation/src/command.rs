//! Player commands — the atomic inputs that drive the simulation.
//!
//! Commands reference entities by [`SimulationId`] rather than Bevy's [`Entity`] so they
//! are identical across all peers and survive serialization to replay files.

use ferrets_math::{fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use crate::simulation_id::SimulationId;

/// A player command.
#[derive(Debug, Clone)]
pub enum PlayerCommand {
    /// Replaces the player's selection with the given entity.
    SelectById { id: SimulationId },
    /// Replaces the player's selection with all entities inside `rect`.
    SelectByRect { rect: FixedURect },
    /// Issues a move order to the current selection, targeting `target`.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Move { target: FixedUVec2, flush: bool },
    /// Issues an attack order to the current selection, targeting the entity `target`.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Attack { target: SimulationId, flush: bool },
    /// Sends the current selection to the entity `target`, resolving the intent per
    /// unit: harvest from a source, deliver to a storage, attack an enemy, etc.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    SendToEntity { target: SimulationId, flush: bool },
    /// Enqueues one unit of `type_name` for production on the `trainer` entity.
    TrainEntity {
        trainer: SimulationId,
        type_name: String,
    },
    /// Issues a build order to the `builder` entity: construct a building of
    /// `type_name` at `position`.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    BuildEntity {
        builder: SimulationId,
        type_name: String,
        position: FixedUVec2,
        flush: bool,
    },
    /// Stops the current orders.
    Stop,
}
