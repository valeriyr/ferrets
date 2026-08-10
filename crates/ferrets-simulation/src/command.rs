//! Player commands — the atomic inputs that drive the simulation.
//!
//! Commands reference entities by [`SimulationId`] rather than Bevy's `Entity` so they
//! are identical across all peers and survive serialization to replay files.

use ferrets_math::{fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use serde::{Deserialize, Serialize};

use crate::{
    components::{rally::RallyTarget, stance::Stance},
    content::{research::ResearchId, skills::SkillId},
    order::AttackTarget,
    simulation_id::SimulationId,
};

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

/// Who performs a cast: the issuing player itself, or one of its entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillCasterRef {
    /// The issuing player casts; its identity comes from the input frame that
    /// carried the command, never from the payload.
    Player,
    /// The given entity casts. It must be owned by the issuing player and its
    /// type must declare the skill.
    Entity(SimulationId),
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
    /// Starts the given research on the `researcher` entity.
    StartResearch {
        researcher: SimulationId,
        research: ResearchId,
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
    /// Issues a repair order against `target` to every selected entity that can
    /// mend it.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Repair { target: SimulationId, flush: bool },
    /// Issues a follow order to the current selection: stay close to `target`,
    /// chasing it as it moves — the explicit form of what a send-to-entity falls
    /// back to, for when an earlier reading (boarding, repairing) would win.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Follow { target: SimulationId, flush: bool },
    /// Issues a board order to every selected entity: ride inside the `target`
    /// transporter — the explicit form of the send-to-entity reading, for when
    /// an earlier reading (repairing a damaged holder) would win.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Board { target: SimulationId, flush: bool },
    /// Issues a load order to the `transport` entity: fetch `target` aboard.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Load {
        transport: SimulationId,
        target: SimulationId,
        flush: bool,
    },
    /// Issues an unload order to the `transport` entity: let every passenger out,
    /// walking into unload range of `at` first when one is given.
    /// `flush` cancels existing orders before issuing this one; `false` appends.
    Unload {
        transport: SimulationId,
        at: Option<FixedUVec2>,
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
    /// Casts `skill` with `caster`, on `target` when the skill needs one
    /// (`None` for a self-targeted skill).
    UseSkill {
        skill: SkillId,
        caster: SkillCasterRef,
        target: Option<SimulationId>,
    },
}
