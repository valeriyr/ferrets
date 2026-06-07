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
    /// Stops the current orders.
    Stop,
}
