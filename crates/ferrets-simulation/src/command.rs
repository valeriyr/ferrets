//! Player commands — the atomic inputs that drive the simulation.
//!
//! Commands reference entities by [`SimulationId`] rather than Bevy's `Entity` so they
//! are identical across all peers and survive serialization to replay files.

use ferrets_math::{fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use serde::{Deserialize, Serialize};

use crate::components::rally::RallyTarget;
use crate::components::stance::Stance;
use crate::simulation_id::SimulationId;

/// A player command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Issues an attack-move order to the current selection, targeting `target`:
    /// move there, engaging hostiles noticed on the way.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    AttackMove { target: FixedUVec2, flush: bool },
    /// Issues a patrol order to the current selection: walk back and forth
    /// between each unit's current position and `target`, engaging hostiles
    /// noticed on the way, until cancelled.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Patrol { target: FixedUVec2, flush: bool },
    /// Issues a guard order to the current selection: stay near the entity
    /// `target` and engage hostiles that threaten it or come close.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Guard { target: SimulationId, flush: bool },
    /// Sets the stance of the current selection.
    SetStance { stance: Stance },
    /// Sends the current selection to the entity `target`, resolving the intent per
    /// unit: harvest from a source, deliver to a storage, attack an enemy, etc.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    SendToEntity { target: SimulationId, flush: bool },
    /// Enqueues one unit of `type_name` for production on the `trainer` entity.
    TrainEntity {
        trainer: SimulationId,
        type_name: String,
    },
    /// Sets or clears the rally point of the `entity`: units it emits take an
    /// order toward the target when they spawn. `None` clears.
    SetRallyPoint {
        entity: SimulationId,
        target: Option<RallyTarget>,
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
    /// Spawns a fully-formed entity of `type_name` for the issuing player at
    /// `position`, bypassing production. A sandbox/debug and scenario-scripting
    /// command — it runs through the normal command pipeline (so it stays
    /// deterministic and replay-safe), unlike scenario setup which spawns
    /// directly.
    Spawn {
        type_name: String,
        position: FixedUVec2,
    },
}
