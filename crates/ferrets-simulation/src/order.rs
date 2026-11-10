//! Internal resolved orders — what entity systems actually execute each tick.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::{command::SkillTarget, simulation_id::SimulationId};
use ferrets_content::{research::ResearchId, skills::SkillId};

/// What an attack is aimed at.
///
/// A weapon whose projectile follows its target is aimed at an entity; one that
/// sends its shot to a cell is aimed at that cell, and hits whatever is
/// standing there when it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackTarget {
    /// The entity with the given id.
    Entity(SimulationId),
    /// A cell.
    Cell(CellPos),
}

impl AttackTarget {
    /// The aimed entity, or `None` for a cell.
    #[inline]
    pub fn entity(self) -> Option<SimulationId> {
        match self {
            AttackTarget::Entity(id) => Some(id),
            AttackTarget::Cell(_) => None,
        }
    }
}

/// Bounds an automatic engagement: the fight ends when the target strays beyond a
/// fixed distance of where the engagement began.
#[derive(Debug, Clone, Copy)]
pub struct Leash {
    /// The position the engagement started from.
    pub position: FixedUVec2,
    /// How far from that position the target may stray, in grid cells, before
    /// the fight is broken off.
    pub radius: u32,
}

/// What a builder is set to do on a site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTask {
    /// Raise a new site of `type_name` anchored at `anchor`: the builder is
    /// its founder.
    Found {
        /// The type the site is raised as.
        type_name: String,
        /// The cell the site's footprint is anchored at.
        anchor: CellPos,
    },
    /// Join the crew of the site already under way with the given id.
    Join {
        /// The site.
        site: SimulationId,
    },
}

impl BuildTask {
    /// Raise a new site of `type_name` anchored at `anchor`.
    pub fn found(type_name: &str, anchor: CellPos) -> Self {
        BuildTask::Found {
            type_name: type_name.to_string(),
            anchor,
        }
    }

    /// Join the crew of the site `site` under way.
    pub fn join(site: SimulationId) -> Self {
        BuildTask::Join { site }
    }
}

/// Where a walk ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveGoal {
    /// A place the walk itself is for: the walker's middle on the point, held
    /// on the map for its size, or near it when a crowd holds the point.
    Position(FixedUVec2),
    /// Within `range` cells of a rectangle, the reach another order judges the
    /// walk by: it arrives only there.
    Within {
        /// The cells closed on.
        rect: CellRect,
        /// How near, in whole cells, measured from the walker's footprint;
        /// `0` puts the walker's anchor in the rectangle.
        range: u32,
    },
}

impl MoveGoal {
    /// Whether the walk ends on a point.
    pub fn is_position(self) -> bool {
        match self {
            MoveGoal::Position(_) => true,
            MoveGoal::Within { .. } => false,
        }
    }
}

/// An order an entity is executing or waiting to execute.
#[derive(Debug, Clone)]
pub enum Order {
    /// Walk to `goal`.
    Move { goal: MoveGoal },
    /// Attack what `target` names — an entity, or a cell for a weapon that sends its
    /// shots to one. An entity target ends the order once it is gone or unreachable; a
    /// cell is never gone, so a ground attack keeps firing until it is canceled. A
    /// leash additionally breaks the attack off when the target strays too far —
    /// automatic engagements set one, explicit ones do not.
    Attack {
        target: AttackTarget,
        leash: Option<Leash>,
    },
    /// Move to a world-space position, engaging hostiles noticed on the way
    /// and resuming toward the position after each fight.
    AttackMove { target: FixedUVec2 },
    /// Walk back and forth between the position the order started at and
    /// `target`, engaging hostiles noticed on the way, until canceled.
    Patrol { target: FixedUVec2 },
    /// Stay near the entity with the given id and engage hostiles that
    /// threaten it or come close, until it is gone.
    Guard { target: SimulationId },
    /// Stay within one cell of the entity with the given id, chasing it as it
    /// moves, until it is gone.
    Follow { target: SimulationId },
    /// Work through the entity's train queue, spawning one unit per completed entry.
    Train,
    /// Work on the given research until it completes for the owning player.
    Research { research: ResearchId },
    /// Change into `type_name`, on the terms of the transition the entity's
    /// type declares. The destination is a name because transitions may be
    /// circular — two forms can each name the other (see
    /// [`components::morph::MorphComponent`](crate::components::morph::MorphComponent)).
    Morph { type_name: String },
    /// Construct a building: raise a new site, or join one under way, and work
    /// on it until construction completes.
    Build { task: BuildTask },
    /// Harvest resources in a loop: gather from a source, deliver to a storage,
    /// repeat. `target` is the source or storage the order was issued on.
    Harvest { target: SimulationId },
    /// Mend the entity with the given id: walk to it, then restore its health a
    /// tick at a time until the pool is full, it is gone, or the work can no longer
    /// be paid for.
    Repair { target: SimulationId },
    /// Go into the holder with the given id: walk into its load range, then
    /// go aboard a transporter or through a gateway.
    Board { target: SimulationId },
    /// Take the entity with the given id in: walk into own load range of it,
    /// or send it to board when the holder cannot move, then take it in. The
    /// holder-side mirror of [`Board`](Self::Board).
    Load { target: SimulationId },
    /// Let every body held out, one at a time. With a destination, walk into
    /// unload range of it first and send each freed body marching there;
    /// without one, freed bodies go to the rally point, if set.
    Unload { at: Option<CellPos> },
    /// Cast `skill` at `target`: walk into the skill's reach of what it is
    /// aimed at, then work at it until it lands. A skill that declares no
    /// reach has nothing to walk toward and is worked where the caster
    /// stands.
    Cast {
        skill: SkillId,
        target: Option<SkillTarget>,
    },
    /// Wait out the dying phase, then leave the world.
    Die,
}

impl Order {
    /// If this order is a move order, returns where it goes. Otherwise,
    /// returns `None`.
    pub fn move_goal(&self) -> Option<MoveGoal> {
        match self {
            Order::Move { goal } => Some(*goal),
            _ => None,
        }
    }

    /// If this order is an attack order, returns what it is aimed at. Otherwise,
    /// returns `None`.
    pub fn attack_target(&self) -> Option<AttackTarget> {
        match self {
            Order::Attack { target, .. } => Some(*target),
            _ => None,
        }
    }

    /// If this order is an attack order with a leash, returns the leash.
    /// Otherwise, returns `None`.
    pub fn attack_leash(&self) -> Option<Leash> {
        match self {
            Order::Attack { leash, .. } => *leash,
            _ => None,
        }
    }

    /// If this order is an attack-move order, returns the target position.
    /// Otherwise, returns `None`.
    pub fn attack_move_target(&self) -> Option<FixedUVec2> {
        match self {
            Order::AttackMove { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a patrol order, returns the target position. Otherwise,
    /// returns `None`.
    pub fn patrol_target(&self) -> Option<FixedUVec2> {
        match self {
            Order::Patrol { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a guard order, returns the guarded entity's id.
    /// Otherwise, returns `None`.
    pub fn guard_target(&self) -> Option<SimulationId> {
        match self {
            Order::Guard { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a follow order, returns the target id. Otherwise, returns `None`.
    pub fn follow_target(&self) -> Option<SimulationId> {
        match self {
            Order::Follow { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a build order, returns the builder's task. Otherwise,
    /// returns `None`.
    pub fn build_task(&self) -> Option<&BuildTask> {
        match self {
            Order::Build { task } => Some(task),
            _ => None,
        }
    }

    /// If this order is a harvest order, returns the target id. Otherwise, returns `None`.
    pub fn harvest_target(&self) -> Option<SimulationId> {
        match self {
            Order::Harvest { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a repair order, returns the target id. Otherwise, returns `None`.
    pub fn repair_target(&self) -> Option<SimulationId> {
        match self {
            Order::Repair { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a board order, returns the transporter's id. Otherwise,
    /// returns `None`.
    pub fn board_target(&self) -> Option<SimulationId> {
        match self {
            Order::Board { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a load order, returns the fetched entity's id.
    /// Otherwise, returns `None`.
    pub fn load_target(&self) -> Option<SimulationId> {
        match self {
            Order::Load { target } => Some(*target),
            _ => None,
        }
    }

    /// If this order is a cast order, returns the skill and what it is aimed
    /// at — the outer `Option` says whether this is a cast order at all, the
    /// inner one whether the cast names an aim. Otherwise, returns `None`.
    pub fn cast_params(&self) -> Option<(SkillId, Option<SkillTarget>)> {
        match self {
            Order::Cast { skill, target } => Some((*skill, *target)),
            _ => None,
        }
    }

    /// If this order is an unload order, returns its destination — the outer
    /// `Option` says whether this is an unload order at all, the inner one
    /// whether it names a destination. Otherwise, returns `None`.
    pub fn unload_at(&self) -> Option<Option<CellPos>> {
        match self {
            Order::Unload { at } => Some(*at),
            _ => None,
        }
    }
}
