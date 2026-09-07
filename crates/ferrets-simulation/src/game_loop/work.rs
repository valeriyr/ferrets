//! What a worker's declared presence means for it while it works.
//!
//! Every order that puts a worker on a job for a stretch of ticks takes it off
//! the grid and puts it back the same way; both halves of that pairing live
//! here.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

use crate::spawn;
use ferrets_content::work::WorkPresence;

/// Puts a worker where its presence has it stand as it takes up `job`: off
/// the map for one it disappears into, in one of the job's berths for one it
/// attaches to — both free any of the job's cells it was standing on. One that
/// works in the open stays exactly where it walked to and is never moved on
/// its own account.
pub(super) fn enter(world: &mut World, entity: Entity, presence: &WorkPresence, job: Entity) {
    match presence {
        WorkPresence::Hidden => spawn::hide_entity(world, entity),
        WorkPresence::Attached(attachment) => spawn::attach_entity(world, entity, job, attachment),
        WorkPresence::Present | WorkPresence::PresentStacking => {}
    }
}

/// Brings a worker that left the grid for its job back onto it beside the
/// footprint at `around`, and leaves one that worked in the open exactly where
/// it stands.
///
/// A return with no free cell to take is queued for a later tick, not retried
/// here.
pub(super) fn leave(world: &mut World, entity: Entity, around: CellPos, around_size: CellSize) {
    spawn::place_back_near_or_retry(world, entity, around, around_size);
}
