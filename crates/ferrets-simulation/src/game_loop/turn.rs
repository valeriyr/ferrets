//! Coming round: how far a body's look follows what it is doing in one tick.
//!
//! A look is a rate, not an assignment. The body that has just been told to walk
//! the other way is still looking the way it was, and it arrives at the new
//! bearing over as many ticks as its own rates allow — which is what makes a
//! catapult a catapult and a peasant a peasant.

use bevy_ecs::prelude::*;
use ferrets_content::entity_stats::EntityStatId;
use ferrets_math::{
    FixedU64,
    facing::{self, Facing},
};

use crate::{components::location::LocationComponent, entity_def};

/// How far off the way it is going a body may look while walking on, once it has
/// planted its feet to come round.
///
/// A separate, much smaller angle than the one that stops it: released at the
/// angle that stopped it, a body would set off again at full speed still looking
/// a quarter turn away from where it walks, and held to the exact bearing it
/// would stop dead at every corner of every path. A lean this small reads as a
/// body leading into its turn.
const RELEASE_DEGREES: FixedU64 = FixedU64::lit("22.5");

/// Which of a body's two rates one tick of coming round is measured by.
pub(super) enum Rate {
    /// The body is walking: its look follows the way it is going.
    Walking,
    /// The body is standing: it is coming round on the spot.
    Standing,
}

impl Rate {
    /// The stat this rate reads.
    fn stat(&self) -> EntityStatId {
        match self {
            Rate::Walking => EntityStatId::TURN_RATE,
            Rate::Standing => EntityStatId::PIVOT_RATE,
        }
    }
}

/// Turns `entity`'s look toward `wanted` by no more than one tick at `rate`, and
/// answers where it now looks.
pub(super) fn toward(world: &mut World, entity: Entity, wanted: Facing, rate: Rate) -> Facing {
    let allowance = units(world, entity, rate.stat());
    let mut entity_mut = world.entity_mut(entity);
    let mut location = entity_mut
        .get_mut::<LocationComponent>()
        .expect("a turning entity has a location");
    location.facing = location.facing.turn_toward(wanted, allowance);
    location.facing
}

/// Whether the body holds still to come round toward `wanted` instead of walking
/// on, given whether it was already holding.
///
/// Two angles rather than one, and the wider of them only starts the hold: see
/// [`RELEASE_DEGREES`]. A body with no pivot angle never holds — it walks and the
/// look catches up — and neither does one that could not finish the turn, since a
/// rate folded to nothing would leave it standing there for good.
pub(super) fn pivoting(world: &World, entity: Entity, wanted: Facing, already: bool) -> bool {
    let Some(angle) = entity_def::effective_stat(world, entity, EntityStatId::PIVOT_ANGLE) else {
        return false;
    };
    if units(world, entity, EntityStatId::PIVOT_RATE) == 0 {
        return false;
    }
    let facing = world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("a turning entity has a location")
        .facing;
    let trigger = facing::units_of_degrees(angle);
    let release = facing::units_of_degrees(RELEASE_DEGREES).min(trigger / 2);
    let off = facing.distance(wanted);
    if already {
        off > release
    } else {
        off > trigger
    }
}

/// One of `entity`'s angle stats in angle units. A body that declares none turns
/// as far as it likes: an absent limit is no limit.
pub(super) fn units(world: &World, entity: Entity, stat: EntityStatId) -> u32 {
    entity_def::effective_stat(world, entity, stat).map_or(u32::MAX, facing::units_of_degrees)
}
