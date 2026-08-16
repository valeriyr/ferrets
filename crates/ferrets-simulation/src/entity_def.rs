//! Resolution from a spawned entity to what the simulation knows about it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::{
    components::{
        entity_info::EntityInfoComponent, entity_stats::StatsComponent, location::LocationComponent,
    },
    simulation_id::SimulationId,
};
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, registry::ContentRegistry,
};

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

/// The body radius of a mover, from its radius stat.
///
/// Panics if `entity` is not a simulation entity, or carries no radius stat
/// — building a continuous-model map validates that every mover defines
/// one, and only the continuous model reads it.
pub fn radius(world: &World, entity: Entity) -> FixedU64 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(EntityStatId::RADIUS))
        .expect("movers define a radius stat")
}

/// The current effective value of one of `entity`'s stats, or `None` when it
/// carries no such stat.
pub fn effective_stat(world: &World, entity: Entity, stat: EntityStatId) -> Option<FixedU64> {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(stat))
}

/// The whole-number value of one of `entity`'s stats.
///
/// Panics if `entity` is not a simulation entity, or carries no such stat —
/// for stats whose presence the caller's capability check already vouches for.
pub fn effective_stat_u32(world: &World, entity: Entity, stat: EntityStatId) -> u32 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("simulation entity must have a stat store")
        .effective_as_u32(stat)
        .expect("the capability pairs the entity with this stat")
}

/// Returns where `entity` stands and how much room it takes.
///
/// The two belong together — a footprint is neither without the other — and they
/// come from different places: the position is per-entity state, the size is its
/// type's. Anything measuring against an entity wants both, so it asks once.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint(world: &World, entity: Entity) -> (FixedUVec2, CellSize) {
    let size = of(world, entity)
        .location
        .expect("validated content defines a location")
        .size();
    (position(world, entity), size)
}

/// The footprint `entity` stands on as a rect of whole cells, anchored at
/// its floored position — the value every rect-to-rect measure takes.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint_rect(world: &World, entity: Entity) -> CellRect {
    let (position, size) = footprint(world, entity);
    CellRect::new(CellPos::from(position), size)
}

/// The center of the footprint `entity` stands on, in world units with
/// sub-cell precision.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint_center(world: &World, entity: Entity) -> FixedUVec2 {
    let (position, size) = footprint(world, entity);
    FixedUVec2::new(
        position.x + FixedU64::from_num(size.width) / 2,
        position.y + FixedU64::from_num(size.height) / 2,
    )
}
