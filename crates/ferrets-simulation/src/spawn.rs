//! Simulation entity creation from registered type definitions.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};

use crate::{
    components::{
        entity_info::EntityInfoComponent, location::LocationComponent,
        order_queue::OrderQueueComponent,
    },
    content::registry::ContentRegistry,
    map::Map,
    simulation_id::{SimulationId, SimulationIdGenerator},
};

/// Spawns an entity of the given type at `position`.
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not registered
/// or the position is blocked on the nav grid.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
) -> Option<(Entity, SimulationId)> {
    let (location_data, move_data) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name)?;
        (type_def.location, type_def.movement)
    };

    let location = LocationComponent::new(position, FixedVec2::ZERO);

    {
        let map = world.resource::<Map>();
        if !map.can_place_entity(&location, &location_data) {
            return None;
        }
    }

    let id = world.resource_mut::<SimulationIdGenerator>().generate();

    let mut entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_name),
        location,
        location_data,
        OrderQueueComponent::default(),
    ));
    if let Some(move_data) = move_data {
        entity_mut.insert(move_data);
    }
    let entity = entity_mut.id();

    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_data);

    Some((entity, id))
}
