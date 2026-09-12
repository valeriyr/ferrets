//! Resolution from a spawned entity to what the simulation knows about it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use crate::{
    annex,
    components::{
        attached::AttachedComponent, build::UnderConstructionComponent,
        entity_info::EntityInfoComponent, entity_stats::StatsComponent, hidden::HiddenComponent,
        location::LocationComponent, morph::MorphComponent, order_queue::OrderQueueComponent,
        owner::OwnerComponent,
    },
    fields,
    map::OccupancyClass,
    order::Order,
    session::player_id::PlayerId,
    simulation_id::SimulationId,
};
use ferrets_content::{
    annex::AnnexDef,
    attack::Weapon,
    build::BuilderAttendance,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    morph::MorphTransition,
    period::Period,
    registry::ContentRegistry,
    resource::HarvestData,
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

/// How `entity` relates to a site it raises, or `None` when its type cannot
/// build.
pub fn builder_attendance(world: &World, entity: Entity) -> Option<&BuilderAttendance> {
    of(world, entity)
        .builder
        .as_ref()
        .map(|builder| builder.attendance())
}

/// The annex terms `entity`'s type declares, or `None` when it is no annex.
pub fn annex_def(world: &World, entity: Entity) -> Option<AnnexDef> {
    of(world, entity).annex
}

/// How `entity` makes a trip for `kind`, or `None` when its type does not
/// carry it.
pub fn harvest_data<'w>(world: &'w World, entity: Entity, kind: &str) -> Option<&'w HarvestData> {
    of(world, entity)
        .resource_carrier
        .as_ref()
        .and_then(|carrier| carrier.harvest_data(kind))
}

/// Whether `entity` holds its footprint's cells on the navigation grid: it is
/// neither hidden nor attached to a job.
pub fn stands_on_grid(world: &World, entity: Entity) -> bool {
    let entity_ref = world.entity(entity);
    !entity_ref.contains::<HiddenComponent>() && !entity_ref.contains::<AttachedComponent>()
}

/// What switched a standing entity off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// A field effect over the ground it stands on.
    Field,
    /// It is an annex with no primary, on terms that stop it working.
    Alone,
}

/// Whether an entity is in a state to carry out its type's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Stands finished and undisturbed.
    Operating,
    /// Still being raised.
    UnderConstruction,
    /// Standing, but switched off.
    Disabled(Switch),
}

/// Whether a change of form is under way on `entity`: it wears its origin or
/// an interim form until the change lands. Apart from its [`Operation`] — a
/// switched-off entity changes form all the same, and one changing form
/// still operates.
pub fn changing(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<MorphComponent>()
}

/// Returns the [`Operation`] `entity` is in. A site still being raised is
/// [`Operation::UnderConstruction`] whatever the fields say about it or the
/// dock it stands in.
pub fn operation(world: &World, entity: Entity) -> Operation {
    let entity_ref = world.entity(entity);
    if entity_ref.contains::<UnderConstructionComponent>() {
        Operation::UnderConstruction
    } else if fields::disabled(world, entity) {
        Operation::Disabled(Switch::Field)
    } else if annex::idles_alone(world, entity) {
        Operation::Disabled(Switch::Alone)
    } else {
        Operation::Operating
    }
}

/// Every order in `entity`'s queue, front first, sub-orders included. Empty
/// for an entity with no queue.
pub fn orders(world: &World, entity: Entity) -> Vec<Order> {
    world
        .entity(entity)
        .get::<OrderQueueComponent>()
        .map_or_else(Vec::new, |queue| {
            queue.0.iter().map(|entry| entry.order.clone()).collect()
        })
}

/// The terms of the change of form `morph` is under, read from the type that
/// declared it — which the entity may no longer be, when it wears an interim
/// form.
pub fn morph_terms(world: &World, morph: &MorphComponent) -> Option<MorphTransition> {
    world
        .resource::<ContentRegistry>()
        .def(morph.from)
        .morphs
        .iter()
        .find(|transition| transition.into_type() == morph.into)
        .cloned()
}

/// The form a change on `entity` is declared on: the one it changes from
/// when a change is under way, its own otherwise.
pub fn morph_origin(world: &World, entity: Entity) -> EntityTypeId {
    world
        .entity(entity)
        .get::<MorphComponent>()
        .map_or_else(|| type_id(world, entity), |morph| morph.from)
}

/// Ticks a declared period comes to for `entity`: a constant is what it says,
/// and a stat names the entity's effective value. A period of zero is due the
/// tick it starts.
pub fn period_ticks(world: &World, entity: Entity, period: Period) -> u32 {
    match period {
        Period::Constant(ticks) => ticks,
        Period::Stat(id) => effective_stat(world, entity, id)
            .map(|time| time.to_num::<u32>())
            .unwrap_or(0),
    }
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

/// The footprint `entity` stands on as a rect of whole cells — what a reach
/// measure takes for the thing being measured against. A standing form's is
/// anchored at the cell its position rounds to ([`body::anchor`]), the cells
/// its footprint is stamped on; a mover's at the cell its position floors
/// into, the one footprint a walk can plan toward.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint_rect(world: &World, entity: Entity) -> CellRect {
    let (position, size) = footprint(world, entity);
    let anchor = match OccupancyClass::of(of(world, entity)) {
        OccupancyClass::Static => body::anchor(position),
        OccupancyClass::Claim => CellPos::from(position),
    };
    CellRect::new(anchor, size)
}

/// The cells `entity`'s footprint occupies: anchored at the cell its position
/// rounds to ([`body::anchor`]), where a standing form is stamped and a
/// mover's claim lies.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn occupied_rect(world: &World, entity: Entity) -> CellRect {
    let (position, size) = footprint(world, entity);
    CellRect::new(body::anchor(position), size)
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
