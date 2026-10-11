//! What a worker's declared presence means for it while it works.
//!
//! Every order that puts a worker on a job for a stretch of ticks takes it off
//! the grid and puts it back the same way; both halves of that pairing live
//! here.

use bevy_ecs::{entity::Entity, world::World};

use super::orders::Refusal;
use crate::{
    berths,
    beside::{Beside, Lean},
    entity_def, spawn,
};
use ferrets_content::work::WorkPresence;

/// Refuses `entity` a job with `presence` that can take it off the map while
/// others sit in its berths ([`Refusal::Busy`]), as boarding is refused it: a
/// hidden presence always leaves the map, a seated one when no cell is free
/// as the job ends.
pub(super) fn requires_free_to_hide(
    world: &World,
    entity: Entity,
    presence: &WorkPresence,
) -> Result<(), Refusal> {
    match presence {
        WorkPresence::Hidden { .. } | WorkPresence::Attached(_) => {
            if berths::seated_by_others(world, entity, &[]) {
                Err(Refusal::Busy)
            } else {
                Ok(())
            }
        }
        WorkPresence::Present { .. } => Ok(()),
    }
}

/// Puts a worker where its presence has it stand as it takes up `job`: off
/// the map for one it disappears into, in one of the job's berths for one it
/// attaches to — both free any of the job's cells it was standing on. One that
/// works in the open stays exactly where it walked to and is never moved on
/// its own account.
pub(super) fn enter(world: &mut World, entity: Entity, presence: &WorkPresence, job: Entity) {
    match presence {
        WorkPresence::Hidden { .. } => spawn::hide_entity(world, entity),
        WorkPresence::Attached(attachment) => spawn::attach_entity(world, entity, job, attachment),
        WorkPresence::Present { .. } => {}
    }
}

/// Brings a worker that left the grid for its job back onto it: one off the
/// map beside `beside`, on the free cell nearest where it went in; one seated
/// in a berth on the free cell nearest its berth. One that worked in the open
/// is left exactly where it stands.
///
/// A return with no free cell to take is queued for a later tick, not retried
/// here; a seated worker's queued return then searches beside `beside` as a
/// hidden one's does. A dying worker is left where it is: a death off the map
/// finishes there, and a seated worker dies in its seat.
pub(super) fn leave(world: &mut World, entity: Entity, beside: Beside) {
    if entity_def::dying(world, entity) {
        return;
    }
    spawn::place_back_beside_or_retry(world, entity, beside, Lean::WentIn);
}
