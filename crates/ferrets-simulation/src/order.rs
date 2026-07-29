//! Internal resolved orders — what entity systems actually execute each tick.

use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// Bounds an automatic engagement: the fight ends when the target strays beyond a
/// fixed distance of where the engagement began.
#[derive(Debug, Clone, Copy)]
pub struct Leash {
    /// The position the engagement started from.
    pub anchor: FixedUVec2,
    /// How far from the anchor the target may stray, in grid cells, before the
    /// fight is broken off.
    pub radius: u32,
}

/// An order an entity is executing or waiting to execute.
#[derive(Debug, Clone)]
pub enum Order {
    /// Move to within `range` grid cells of a world-space position in simulation
    /// coordinates. `range` of `0` requires reaching the exact cell.
    Move { target: FixedUVec2, range: u32 },
    /// Attack the entity with the given id until it dies or becomes
    /// unreachable. A leash additionally breaks the attack off when the target
    /// strays too far — automatic engagements set one, explicit ones do not.
    Attack {
        target: SimulationId,
        leash: Option<Leash>,
    },
    /// Move to a world-space position, engaging hostiles noticed on the way
    /// and resuming toward the position after each fight.
    AttackMove { target: FixedUVec2 },
    /// Walk back and forth between the position the order started at and
    /// `target`, engaging hostiles noticed on the way, until cancelled.
    Patrol { target: FixedUVec2 },
    /// Stay near the entity with the given id and engage hostiles that
    /// threaten it or come close, until it is gone.
    Guard { target: SimulationId },
    /// Stay within one cell of the entity with the given id, chasing it as it
    /// moves, until it is gone.
    Follow { target: SimulationId },
    /// Work through the entity's train queue, spawning one unit per completed entry.
    Train,
    /// Construct a building of `type_name` at `position`: walk to the site, place
    /// the building, and work until construction completes.
    Build {
        type_name: String,
        position: FixedUVec2,
    },
    /// Harvest resources in a loop: gather from a source, deliver to a storage,
    /// repeat. `target` is the source or storage the order was issued on.
    Harvest { target: SimulationId },
    /// Wait out the dying phase, then leave the world.
    Die,
}

impl Order {
    /// If this order is a move order, returns the target position and range. Otherwise, returns `None`.
    pub fn move_params(&self) -> Option<(FixedUVec2, u32)> {
        match self {
            Order::Move { target, range } => Some((*target, *range)),
            _ => None,
        }
    }

    /// If this order is an attack order, returns the target id. Otherwise, returns `None`.
    pub fn attack_target(&self) -> Option<SimulationId> {
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

    /// If this order is a build order, returns the type name and position. Otherwise, returns `None`.
    pub fn build_params(&self) -> Option<(&str, FixedUVec2)> {
        match self {
            Order::Build {
                type_name,
                position,
            } => Some((type_name, *position)),
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
}
