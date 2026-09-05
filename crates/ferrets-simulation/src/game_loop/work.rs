//! What a worker's declared presence means for it while it works.
//!
//! Every order that puts a worker on a job for a stretch of ticks hides and
//! reveals it the same way; this module keeps that pairing in one place so the
//! verbs cannot drift apart.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

use crate::spawn;
use ferrets_content::work::WorkPresence;

/// Takes a worker off the map for a job its presence says it disappears into,
/// which frees any of the job's cells it was standing on.
///
/// A no-op for one that works in the open, which stays exactly where it walked to
/// and is never moved on its own account.
pub(super) fn enter(world: &mut World, entity: Entity, presence: WorkPresence) {
    if presence.is_hidden() {
        spawn::hide_entity(world, entity);
    }
}

/// Brings a worker that disappeared into its job back out beside the footprint at
/// `around`, and leaves one that worked in the open exactly where it stands.
///
/// The reveal is queued rather than retried, because the callers all finish their
/// order in the same tick and are in no position to retry it themselves.
pub(super) fn leave(world: &mut World, entity: Entity, around: CellPos, around_size: CellSize) {
    spawn::reveal_entity_near_or_retry(world, entity, around, around_size);
}
