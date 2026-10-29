//! Retry step for entities waiting on a free cell to return to the grid.

use bevy_ecs::world::World;

use crate::{
    components::hidden::{self, HiddenComponent, Inside},
    entity_def,
    entity_index::EntityIndex,
    make_way::{self, Asked},
    spawn::{self, SetDown},
};

/// Retries the return of every entity left waiting on a free cell.
///
/// Each tick, every hidden entity whose marker is [`Inside::Waiting`]
/// reattempts its return beside what it names; on success the entity is back
/// on the grid, and the return puts the marker away. With no cell free, the
/// own and allied movers that alone keep it off one are asked to make way.
/// Entities are visited in ascending simulation-id order.
pub fn process_pending_reveals(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let Some(marker) = world.entity(entity).get::<HiddenComponent>() else {
            continue;
        };
        let (beside, lean) = match marker.inside {
            Inside::Waiting { beside, lean, .. } => (beside, lean),
            Inside::Transporter(_) | Inside::Job(_) | Inside::Link(_) | Inside::Nothing => continue,
        };
        // The search beside a container that still stands is noted for when
        // it is gone.
        let searched = entity_def::beside_rect(world, entity, beside);
        hidden::note_last_search(world, entity, searched);
        match spawn::place_back_beside(world, entity, beside, lean) {
            SetDown::Placed => {}
            // No cell yet: it waits for the next tick, while what keeps it
            // off one is asked aside.
            SetDown::NoCell => match make_way::clear_landing(
                world,
                entity_def::owner(world, entity),
                |world| spawn::beside_lander(world, entity, beside, lean),
                &[],
                &[entity],
            ) {
                Asked::Leaves | Asked::Boxed => {}
            },
        }
    }
}
