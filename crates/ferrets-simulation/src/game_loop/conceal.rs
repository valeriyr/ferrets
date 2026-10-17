//! Per-tick refit of the concealment marker from everything that conceals.

use bevy_ecs::world::World;

use crate::{entity_index::EntityIndex, spawn};

/// Fits [`ConcealedComponent`](crate::components::concealed::ConcealedComponent) to every alive entity something conceals this
/// tick — its type, an active buff, or a field it declares an effect for — and
/// takes it off every other.
pub fn refit_concealment(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        spawn::fit_concealment(world, entity);
    }
}
