//! Per-tick refit of the concealment marker from everything that conceals.

use bevy_ecs::world::World;

use crate::{
    components::concealed::ConcealedComponent, entity_def, entity_index::EntityIndex, spawn,
};

/// Fits [`ConcealedComponent`] to every alive entity something conceals this
/// tick — its type, an active buff, or a field it declares an effect for — and
/// takes it off every other.
pub fn refit_concealment(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let concealed = entity_def::concealed(world, entity);
        spawn::fit_default::<ConcealedComponent>(&mut world.entity_mut(entity), concealed);
    }
}
