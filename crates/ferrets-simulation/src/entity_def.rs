//! Resolution from a spawned entity to what the simulation knows about it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use crate::{
    components::{
        entity_info::EntityInfoComponent, entity_stats::StatsComponent,
        location::LocationComponent, owner::OwnerComponent,
    },
    session::player_slot::PlayerId,
    simulation_id::SimulationId,
};
use ferrets_content::{
    attack::Weapon,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    registry::ContentRegistry,
    targeting,
    turret::TurretStats,
};
use ferrets_pathfinder::layer_mask::LayerMask;

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

/// Returns the type handle `entity` currently is.
///
/// Panics if `entity` is not a simulation entity.
pub fn type_id(world: &World, entity: Entity) -> EntityTypeId {
    world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .type_id()
}

/// Returns the player owning `entity`, or `None` for a neutral one.
pub fn owner(world: &World, entity: Entity) -> Option<PlayerId> {
    world
        .entity(entity)
        .get::<OwnerComponent>()
        .map(OwnerComponent::player)
}

/// Returns the [`EntityTypeDef`] for `entity`, resolved through the type handle on
/// its [`EntityInfoComponent`].
///
/// Panics if `entity` is not a simulation entity.
pub fn of(world: &World, entity: Entity) -> &EntityTypeDef {
    world
        .resource::<ContentRegistry>()
        .def(type_id(world, entity))
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

/// How strongly a mover resists displacement, from its weight stat.
///
/// Panics if `entity` is not a simulation entity, or carries no weight stat
/// — a continuous-model map built from map data validates that every mover
/// defines one, and only the continuous model reads it. A map assembled
/// directly from a grid answers for its own content.
pub fn weight(world: &World, entity: Entity) -> FixedU64 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .and_then(|stats| stats.effective(EntityStatId::WEIGHT))
        .expect("movers define a weight stat")
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
/// its floored position — what a reach measure takes for the thing being
/// measured against.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint_rect(world: &World, entity: Entity) -> CellRect {
    let (position, size) = footprint(world, entity);
    CellRect::new(CellPos::from(position), size)
}

/// The cells `entity` stands on, per [`body::standing_rect`] — what a reach
/// measure takes for the one doing the reaching.
///
/// Reach is judged from this to the other side's [`footprint_rect`], never the
/// other way about: the reacher gets the benefit of every cell it stands on,
/// while what it reaches for stays the one footprint a walk can plan toward.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn standing_rect(world: &World, entity: Entity) -> CellRect {
    let (position, size) = footprint(world, entity);
    body::standing_rect(position, size)
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

/// Every layer the weapons `entity` carries reach between them — the weapon its
/// body points and every turret's.
pub fn weapon_targets(world: &World, entity: Entity) -> LayerMask {
    world
        .resource::<ContentRegistry>()
        .targets_of(of(world, entity))
}

/// What the weapon `entity` points itself reaches, and nothing its turrets do.
pub fn body_weapon_targets(world: &World, entity: Entity) -> LayerMask {
    of(world, entity)
        .attack
        .as_ref()
        .map_or(LayerMask::EMPTY, |attack| attack.weapon().targets())
}

/// How far the furthest-reaching weapon `entity` carries can hit.
pub fn weapon_range(world: &World, entity: Entity) -> u32 {
    longest(
        world,
        entity,
        EntityStatId::ATTACK_RANGE,
        |reads| reads.range,
        |_, _| true,
    )
}

/// How far the furthest-noticing weapon `entity` carries engages on its own
/// initiative.
pub fn notice_range(world: &World, entity: Entity) -> u32 {
    longest(
        world,
        entity,
        EntityStatId::ACQUIRE_RANGE,
        |reads| reads.acquire_range,
        |_, _| true,
    )
}

/// How far the furthest-reaching weapon that can serve `target` shoots — every
/// weapon that reaches its layers, or, for a bare cell (`None`), every one whose
/// shots are sent to a place.
///
/// This is the distance an ordered attack closes to, so a body never stops at
/// the reach of a weapon that could not join this fight — an escort with a long
/// gun for the air still walks its short one onto what crawls.
pub fn weapon_range_serving(world: &World, entity: Entity, target: Option<Entity>) -> u32 {
    longest(
        world,
        entity,
        EntityStatId::ATTACK_RANGE,
        |reads| reads.range,
        serves(world, target),
    )
}

/// How far the furthest-noticing weapon that can serve `target` engages on its
/// own initiative — the reach [`weapon_range_serving`] filters, applied to the
/// notice instead.
pub fn notice_range_serving(world: &World, entity: Entity, target: Option<Entity>) -> u32 {
    longest(
        world,
        entity,
        EntityStatId::ACQUIRE_RANGE,
        |reads| reads.acquire_range,
        serves(world, target),
    )
}

/// Whether a weapon can serve a fight against `target`: it reaches the target's
/// layers, or, for a bare cell (`None`), its shots are sent to a place.
fn serves(world: &World, target: Option<Entity>) -> impl Fn(&ContentRegistry, &Weapon) -> bool {
    move |registry, weapon| match target {
        Some(target) => targeting::reaches(weapon.targets(), of(world, target)),
        None => registry.weapon_aims_at_cells(weapon),
    }
}

/// The longest of one number across the weapons `entity` carries that `serves`
/// keeps: what the body as a whole reaches, or notices. The body's own weapon
/// reads `body_reads` — the standard stat, its by definition — and each turret
/// the stat its own definition names, picked by `turret_reads`.
///
/// Zero where none is kept — reachable only when a morph takes the serving
/// weapon away mid-fight. An order on a named target then ends on its every-tick
/// reachability check; one on a bare cell holds at a zero reach, which walks the
/// body no further than adjacency, until it is cancelled.
fn longest(
    world: &World,
    entity: Entity,
    body_reads: EntityStatId,
    turret_reads: impl Fn(TurretStats) -> EntityStatId,
    serves: impl Fn(&ContentRegistry, &Weapon) -> bool,
) -> u32 {
    let registry = world.resource::<ContentRegistry>();
    let def = of(world, entity);
    let body = def
        .attack
        .as_ref()
        .map(|attack| (attack.weapon(), body_reads));
    let turrets = def.turrets.iter().map(|mount| {
        let turret = registry.turret_def(mount.turret());
        (turret.weapon(), turret_reads(turret.stats()))
    });
    body.into_iter()
        .chain(turrets)
        .filter(|(weapon, _)| serves(registry, weapon))
        .map(|(_, stat)| effective_stat_u32(world, entity, stat))
        .max()
        .unwrap_or(0)
}
