//! Resolution from a spawned entity to what the simulation knows about it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::nav_size::NavSize;

use crate::components::entity_info::EntityInfoComponent;
use crate::components::location::LocationComponent;
use crate::content::{entity_type_def::EntityTypeDef, registry::ContentRegistry};
use crate::simulation_id::SimulationId;

/// Returns the [`SimulationId`] `entity` was spawned with.
///
/// Panics if `entity` is not a simulation entity.
pub fn simulation_id(world: &World, entity: Entity) -> SimulationId {
    world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .id()
}

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

/// Returns where `entity` stands.
///
/// Panics if `entity` is not a simulation entity.
pub fn position(world: &World, entity: Entity) -> FixedUVec2 {
    world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("simulation entity must have LocationComponent")
        .position
}

/// Returns where `entity` stands and how much room it takes.
///
/// The two belong together — a footprint is neither without the other — and they
/// come from different places: the position is per-entity state, the size is its
/// type's. Anything measuring against an entity wants both, so it asks once.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint(world: &World, entity: Entity) -> (FixedUVec2, NavSize) {
    let size = of(world, entity)
        .location
        .expect("validated content defines a location")
        .size();
    (position(world, entity), size)
}
