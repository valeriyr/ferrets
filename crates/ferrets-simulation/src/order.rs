//! Internal resolved orders — what entity systems actually execute each tick.

use ferrets_geometry::cell_size::CellSize;
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

use crate::{content::research::ResearchId, simulation_id::SimulationId};

/// What an attack is aimed at.
///
/// A weapon whose projectile follows its target is aimed at an entity; one that sends
/// its shot to a cell is aimed at a position, and hits whatever is standing there when
/// it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackTarget {
    /// The entity with the given id.
    Entity(SimulationId),
    /// A cell, in simulation coordinates.
    Position(FixedUVec2),
}

impl AttackTarget {
    /// The aimed entity, or `None` for a position.
    #[inline]
    pub fn entity(self) -> Option<SimulationId> {
        match self {
            AttackTarget::Entity(id) => Some(id),
            AttackTarget::Position(_) => None,
        }
    }
}

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
    /// Move to within `range` grid cells of the footprint at `target` with the given
    /// `size`, in simulation coordinates. `range` of `0` requires reaching the
    /// footprint itself. A bare cell is a footprint of [`CellSize::ONE`]; naming the
    /// whole of a larger destination is what lets a unit stop at the near side of a
    /// building rather than walking round to the corner its position names.
    Move {
        target: FixedUVec2,
        size: CellSize,
        range: u32,
    },
    /// Attack what `target` names — an entity, or a cell for a weapon that sends its
    /// shots to one. An entity target ends the order once it is gone or unreachable; a
    /// cell is never gone, so a ground attack keeps firing until it is cancelled. A
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
    /// Work on the given research until it completes for the owning player.
    Research { research: ResearchId },
    /// Construct a building of `type_name` at `position`: walk to the site, place
    /// the building, and work until construction completes.
    Build {
        type_name: String,
        position: FixedUVec2,
    },
    /// Harvest resources in a loop: gather from a source, deliver to a storage,
    /// repeat. `target` is the source or storage the order was issued on.
    Harvest { target: SimulationId },
    /// Mend the entity with the given id: walk to it, then restore its health a
    /// tick at a time until the pool is full, it is gone, or the work can no longer
    /// be paid for.
    Repair { target: SimulationId },
    /// Ride inside the transporter with the given id: walk into its load range,
    /// then disappear aboard until it unloads.
    Board { target: SimulationId },
    /// Fetch the entity with the given id aboard: walk into own load range of
    /// it, then take it in. The holder-side mirror of [`Board`](Self::Board).
    Load { target: SimulationId },
    /// Let every passenger out, one at a time. With a destination, walk into
    /// unload range of it first and send each freed passenger marching there;
    /// without one, freed passengers go to the rally point, if set.
    Unload { at: Option<FixedUVec2> },
    /// Wait out the dying phase, then leave the world.
    Die,
}

impl Order {
    /// If this order is a move order, returns the destination footprint and range.
    /// Otherwise, returns `None`.
    pub fn move_params(&self) -> Option<(FixedUVec2, CellSize, u32)> {
        match self {
            Order::Move {
                target,
                size,
                range,
            } => Some((*target, *size, *range)),
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

    /// If this order is an unload order, returns its destination — the outer
    /// `Option` says whether this is an unload order at all, the inner one
    /// whether it names a destination. Otherwise, returns `None`.
    pub fn unload_at(&self) -> Option<Option<FixedUVec2>> {
        match self {
            Order::Unload { at } => Some(*at),
            _ => None,
        }
    }
}
