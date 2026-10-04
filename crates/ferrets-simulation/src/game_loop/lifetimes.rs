//! Timed lives, aged each tick.

use bevy_ecs::world::World;
use ferrets_content::entity_stats::EntityStatId;

use crate::{
    components::lifetime::LifetimeComponent, entity_def, entity_index::EntityIndex,
    events::DeathCause, spawn,
};

/// Ages every timed life by one tick, ending the ones whose time is up.
///
/// Runs over the alive index, so the dying are already excluded. The age is
/// compared against the *effective* stat, so a buff that lengthens a life keeps
/// standing instances on their feet and one that shortens it takes them at
/// once.
pub fn process_lifetimes(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let Some(lifetime) = world.entity(entity).get::<LifetimeComponent>() else {
            continue;
        };
        // A timed life is only ever fitted from the stat, so anything carrying
        // one carries the stat.
        let limit = entity_def::effective_ticks(world, entity, EntityStatId::LIFETIME);
        let age = lifetime.age + 1;
        if age >= limit {
            spawn::despawn_entity(world, entity, DeathCause::Expired);
            continue;
        }
        world
            .entity_mut(entity)
            .get_mut::<LifetimeComponent>()
            .expect("the timed life read a moment ago is still the entity's own")
            .age = age;
    }
}
