//! Player commands — the atomic inputs that drive the simulation.
//!
//! Commands reference entities by [`SimulationId`] rather than Bevy's `Entity` so they
//! are identical across all peers and survive serialization to replay files.

use ferrets_math::{fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use serde::{Deserialize, Serialize};

use crate::components::rally::RallyTarget;
use crate::components::stance::Stance;
use crate::content::skills::SkillId;
use crate::order::AttackTarget;
use crate::simulation_id::SimulationId;

/// How a selection command combines with the player's existing selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectMode {
    /// Clears the selection, then selects the resolved entities.
    Replace,
    /// Adds the resolved entities to the selection, skipping any already present.
    Add,
    /// Flips each resolved entity's membership: selected becomes unselected and vice versa.
    Toggle,
    /// Removes the resolved entities from the selection.
    Remove,
}

/// A player command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerCommand {
    /// Selects the given entity, combining with the current selection per `mode`.
    SelectById { id: SimulationId, mode: SelectMode },
    /// Selects all entities inside `rect` sharing the selection class `class`
    /// (see [`EntityTypeDef::selection_class`](crate::content::entity_type_def::EntityTypeDef::selection_class)),
    /// combining with the current selection per `mode`.
    SelectByType {
        class: String,
        rect: FixedURect,
        mode: SelectMode,
    },
    /// Selects all entities inside `rect`, combining with the current selection per `mode`.
    SelectByRect { rect: FixedURect, mode: SelectMode },
    /// Saves the player's current selection as control `group`,
    /// replacing whatever it held.
    AssignGroup { group: u8 },
    /// Adds the player's current selection to control `group`.
    AppendGroup { group: u8 },
    /// Selects control `group`, combining with the current selection per `mode`.
    RecallGroup { group: u8, mode: SelectMode },
    /// Issues a move order to the current selection, targeting `target`.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Move { target: FixedUVec2, flush: bool },
    /// Issues an attack order to the current selection, targeting the entity `target`.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Attack { target: AttackTarget, flush: bool },
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
    /// Uses the `skill` of the `caster` entity, on `target` when the skill needs
    /// one (`None` for a self-targeted skill).
    UseSkill {
        caster: SimulationId,
        skill: SkillId,
        target: Option<SimulationId>,
    },
}
