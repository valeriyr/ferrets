//! Resolution from a spawned entity to its content definition.

use bevy_ecs::{entity::Entity, world::World};

use crate::components::entity_info::EntityInfoComponent;
use crate::content::{entity_type_def::EntityTypeDef, registry::ContentRegistry};

/// Returns the [`EntityTypeDef`] for `entity`, resolved through the type handle on
/// its [`EntityInfoComponent`].
///
/// Panics if `entity` is not a simulation entity.
pub fn of(world: &World, entity: Entity) -> &EntityTypeDef {
    let type_id = world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .type_id();
    world.resource::<ContentRegistry>().def(type_id)
}
