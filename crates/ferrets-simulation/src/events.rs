//! What the simulation announces as it runs.
//!
//! Emitting systems say what happened and nothing about who cares; the consumers
//! reading the record decide that for themselves. The events carry the owners
//! and the position a consumer needs to make that judgement.
//!
//! Everything here is deterministic simulation state: reproduced exactly on
//! playback like anything else the tick produces, and not folded into the
//! checksum.

use bevy_ecs::prelude::*;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use ferrets_content::{
    costs::Cost, entity_type_def::EntityTypeId, research::ResearchId, skills::SkillId,
};

use crate::{session::player_id::PlayerId, simulation_id::SimulationId};

/// How an entity came to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnCause {
    /// One of the map's declared placements, standing before the game begins.
    Placed,
    /// Finished in a producer's queue.
    Trained {
        /// The producer it came out of.
        trainer: SimulationId,
    },
    /// Founded as a construction site — announced when the site is raised,
    /// standing but unfinished. [`SimulationEvent::ConstructionCompleted`]
    /// announces the other end of the work.
    Founded {
        /// Whoever placed the site.
        builder: SimulationId,
    },
    /// Conjured by the sandbox spawn command, with no production behind it.
    Sandbox,
    /// What an entity left behind when it finished dying.
    Remains {
        /// The entity that died here.
        of: SimulationId,
    },
}

/// What a player was charged for.
///
/// The same vocabulary carries a charge and its reversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendCause {
    /// Raising a construction site.
    Construction {
        /// The building being put up.
        site: SimulationId,
    },
    /// A unit queued at a producer.
    Training {
        /// The producer it was queued at.
        trainer: SimulationId,
    },
    /// A research topic.
    Research {
        /// The topic paid for.
        research: ResearchId,
    },
    /// Changing an entity's form.
    Morph {
        /// The entity changing.
        entity: SimulationId,
    },
    /// A skill going off, whether an entity's or the player's own.
    Skill {
        /// The skill paid for.
        skill: SkillId,
    },
    /// Mending something, billed as the work is done.
    Repair {
        /// What is being mended.
        target: SimulationId,
    },
}

/// Why an entity stopped existing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathCause {
    /// Brought to zero health by damage.
    Killed {
        /// Who landed the killing hit.
        by: SimulationId,
        /// That attacker's owner — absent for a neutral attacker, and for one
        /// already gone by the time its shot landed.
        by_owner: Option<PlayerId>,
    },
    /// A resource source that ran out.
    Depleted,
    /// Called off by its owner before it was finished.
    Cancelled,
    /// Consumed by the construction site it founded.
    Consumed,
    /// Went down with the carrier holding it.
    PassengerLost {
        /// The carrier it was inside.
        holder: SimulationId,
    },
}

/// Something the simulation announced.
///
/// Positions travel on the events that have one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulationEvent {
    /// An entity entered the world.
    EntitySpawned {
        /// The new entity.
        entity: SimulationId,
        /// How it got there.
        cause: SpawnCause,
    },
    /// A construction site finished its work and stands as a working building.
    ///
    /// The building itself entered the world when its site was founded (a spawn
    /// with [`SpawnCause::Founded`]); this announces the moment it became whole.
    ConstructionCompleted {
        /// The building that finished.
        building: SimulationId,
        /// Whoever completed the last of the work.
        builder: SimulationId,
    },
    /// An entity stopped existing.
    EntityDied {
        /// The entity that went.
        entity: SimulationId,
        /// What it was.
        entity_type: EntityTypeId,
        /// Whose it was, absent for an unowned one.
        owner: Option<PlayerId>,
        /// Where it went down.
        position: FixedUVec2,
        /// Why.
        cause: DeathCause,
    },
    /// A hit was applied, after armor.
    DamageLanded {
        /// Who took it.
        target: SimulationId,
        /// Whose the victim is.
        target_owner: Option<PlayerId>,
        /// Who dealt it.
        attacker: SimulationId,
        /// That attacker's owner.
        attacker_owner: Option<PlayerId>,
        /// Health actually removed, after armor and before any overkill is
        /// discarded.
        amount: FixedU64,
        /// Where the hit resolved — the victim's own place.
        position: FixedUVec2,
    },
    /// A carried load reached a stockpile.
    ResourcesGathered {
        /// Who banked it.
        player: PlayerId,
        /// Which resource, by content name.
        kind: String,
        /// How much.
        amount: u32,
        /// The stockpile it was banked at.
        storage: SimulationId,
    },
    /// A player was charged.
    ResourcesSpent {
        /// Who paid.
        player: PlayerId,
        /// The whole price, every kind of it — one act of paying, announced once.
        cost: Cost,
        /// What the charge was for.
        cause: SpendCause,
    },
    /// A charge was given back — a cancelled order, or one that turned out to be
    /// impossible after it had already paid.
    ResourcesRefunded {
        /// Who was paid back.
        player: PlayerId,
        /// The whole amount given back.
        cost: Cost,
        /// The charge being reversed.
        cause: SpendCause,
    },
    /// A research topic finished.
    ResearchCompleted {
        /// Who researched it.
        player: PlayerId,
        /// What finished.
        research: ResearchId,
        /// What worked it, absent when something granted it outright.
        researcher: Option<SimulationId>,
    },
    /// An entity became a different type in place.
    ///
    /// Neither a spawn nor a death: the subject is the same entity throughout.
    /// Every landing announces, an interim form's included, so a change that
    /// passes through one is two of these, and a change called off in it a
    /// third.
    EntityMorphed {
        /// The entity that changed.
        entity: SimulationId,
        /// What it was; what it now is, the entity itself says.
        from: EntityTypeId,
    },
    /// An entity went off the map without dying — inside a carrier, a mine, or
    /// whatever else swallows it — and is still there to come back.
    ///
    /// Paired with [`EntityRevealed`](Self::EntityRevealed).
    EntityHidden {
        /// The entity that went away.
        entity: SimulationId,
    },
    /// An entity that was off the map came back to it.
    EntityRevealed {
        /// The entity that returned.
        entity: SimulationId,
    },
    /// An entity's skill went off, having passed its costs and its cooldown.
    SkillCast {
        /// Who cast it.
        caster: SimulationId,
        /// What it was applied to; the caster itself for a self-cast.
        target: SimulationId,
        /// What was cast.
        skill: SkillId,
    },
    /// A player's own skill went off — one with no caster and no target, which
    /// applies to the player itself.
    PlayerSkillCast {
        /// Who cast it.
        player: PlayerId,
        /// What was cast.
        skill: SkillId,
    },
}

/// Everything the current tick has announced, in the order it happened.
///
/// Emission order is deterministic: every peer records the same sequence.
///
/// The record holds one tick's worth: filled while the tick runs, read by every
/// consumer once the tick has finished, then retired in a phase of its own —
/// empty outside the tick that filled it.
#[derive(Resource, Debug, Default)]
pub struct EventRecord {
    announced: Vec<SimulationEvent>,
}

impl EventRecord {
    /// Announces `event`.
    pub fn emit(&mut self, event: SimulationEvent) {
        self.announced.push(event);
    }

    /// Everything announced this tick, in order.
    pub fn events(&self) -> &[SimulationEvent] {
        &self.announced
    }

    /// Drops the tick's announcements.
    pub fn clear(&mut self) {
        self.announced.clear();
    }
}
