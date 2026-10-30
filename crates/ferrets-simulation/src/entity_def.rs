//! Resolution from a spawned entity to what the simulation knows about it.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use crate::{
    annex,
    beside::Beside,
    components::{
        attached::AttachedComponent,
        brood::BredComponent,
        build::OverbuiltComponent,
        dying::{DiedComponent, DyingComponent},
        entity_buffs::BuffsComponent,
        entity_info::EntityInfoComponent,
        entity_stats::StatsComponent,
        hidden::{HiddenComponent, Inside},
        location::LocationComponent,
        morph::MorphComponent,
        movement::MoveComponent,
        order_queue::OrderQueueComponent,
        owner::OwnerComponent,
        pools::PoolsComponent,
        resource::{HarvestingComponent, UnderHarvestComponent},
        rise::{RiseOrigin, RisingComponent},
        transport::BoardedComponent,
    },
    entity_index::EntityIndex,
    fields,
    map::Map,
    movement_model::{self, MovementModel},
    occupancy::{GridMark, OccupancyClass},
    order::Order,
    regions,
    session::player_id::PlayerId,
    simulation_id::SimulationId,
};
use ferrets_content::{
    annex::AnnexDef,
    attack::Weapon,
    build::BuilderAttendance,
    concealment::Concealment,
    entity_buffs::EntityBuffId,
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    location::LocationDef,
    morph::MorphTransition,
    pool_def::PoolId,
    quantity::Quantity,
    registry::ContentRegistry,
    resource::HarvestData,
    targeting,
    turret::TurretStats,
    warp::WarperDef,
};
use ferrets_nav_grid::{layer_id::LayerId, layer_mask::LayerMask, mover_shape::MoverShape};

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

/// The name of the type `entity` is an instance of.
///
/// Panics if `entity` is not a simulation entity.
pub fn type_name(world: &World, entity: Entity) -> &str {
    world
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("simulation entity must have EntityInfoComponent")
        .type_name()
}

/// Returns the player owning `entity`, or `None` for a neutral one.
pub fn owner(world: &World, entity: Entity) -> Option<PlayerId> {
    world
        .entity(entity)
        .get::<OwnerComponent>()
        .map(OwnerComponent::player)
}

/// Whether `other` belongs to whoever owns `entity`; never for an ownerless
/// entity or other.
pub fn same_owner(world: &World, entity: Entity, other: Entity) -> bool {
    match (owner(world, entity), owner(world, other)) {
        (Some(entity_owner), Some(other_owner)) => entity_owner == other_owner,
        (Some(_), None) | (None, _) => false,
    }
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

/// What `entity`'s type warps in, and onto which cells, or `None` when it
/// warps nothing in.
pub fn warper_def(world: &World, entity: Entity) -> Option<&WarperDef> {
    of(world, entity).warper.as_ref()
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

/// Whether `entity` is off the map: aboard a holder, inside a job or a mine,
/// or waiting on a free cell to come back to.
pub fn hidden(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<HiddenComponent>()
}

/// Whether `entity` is in its dying phase.
pub fn dying(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<DyingComponent>()
}

/// Whether `entity` was bred by a breeder and is still tied to it.
pub fn bred(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<BredComponent>()
}

/// Whether `entity`'s dying phase has ended: it is about to be despawned.
pub fn died(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<DiedComponent>()
}

/// Whether `entity` stands on the map, alive: on the grid, with a place.
pub fn stands(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<LocationComponent>()
        && stands_on_grid(world, entity)
        && !dying(world, entity)
}

/// Whether a relocation may move `entity`: standing on the map, finished,
/// raised over no resource source, and not changing form.
pub fn relocatable(world: &World, entity: Entity) -> bool {
    stands(world, entity)
        && !rising(world, entity)
        && !overbuilt(world, entity)
        && !changing(world, entity)
}

/// The slots `passenger` takes in what holds it.
pub fn cargo_size(world: &World, passenger: Entity) -> u32 {
    world
        .entity(passenger)
        .get::<StatsComponent>()
        .expect("a transportable entity has a stat store")
        .effective_as_u32(EntityStatId::CARGO_SIZE)
        .expect("a transportable entity carries cargo_size")
}

/// Whether `entity` was raised over a resource source it covers.
pub fn overbuilt(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<OverbuiltComponent>()
}

/// Whether `entity` sits in a berth of a job, on the map and holding no cells.
pub fn attached(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<AttachedComponent>()
}

/// The job whose berth `entity` sits in, or `None` once it is gone from the
/// world.
///
/// Panics if `entity` sits in no berth.
pub fn berth_job(world: &World, entity: Entity) -> Option<Entity> {
    let job = world
        .get::<AttachedComponent>(entity)
        .expect("only an attached entity sits in a berth")
        .job;
    world.resource::<EntityIndex>().any(job)
}

/// Whether `entity` rides aboard a holder.
pub fn boarded(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<BoardedComponent>()
}

/// Whether `entity` is on a harvest trip.
pub fn harvesting(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<HarvestingComponent>()
}

/// Whether `entity` is a source with a crew on a trip to it.
pub fn under_harvest(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<UnderHarvestComponent>()
}

/// Whether `entity` is still rising into the world.
pub fn rising(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<RisingComponent>()
}

/// What brought `entity` into the world, or `None` once it has risen or when
/// it never rose.
pub fn rise_origin(world: &World, entity: Entity) -> Option<RiseOrigin> {
    world
        .get::<RisingComponent>(entity)
        .map(|rising| rising.origin)
}

/// Whether `entity`'s return to the grid is deferred, waiting on a free cell.
pub fn pending_reveal(world: &World, entity: Entity) -> bool {
    waiting_beside(world, entity).is_some()
}

/// Whether `entity` holds its footprint's cells on the navigation grid: it is
/// neither hidden nor attached to a job.
pub fn stands_on_grid(world: &World, entity: Entity) -> bool {
    let entity_ref = world.entity(entity);
    !entity_ref.contains::<HiddenComponent>() && !entity_ref.contains::<AttachedComponent>()
}

/// The kind of outage a standing entity is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outage {
    /// A field effect over the ground it stands on.
    Field,
    /// A buff it carries.
    Buff,
    /// It is an annex with no primary, on terms that stop it working.
    Orphaned,
}

/// Whether an entity is in a state to carry out its type's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Stands finished and undisturbed.
    Operating,
    /// Still rising into the world.
    Rising,
    /// Standing, but switched off.
    Disabled(Outage),
    /// Off the map, waiting on a free cell to come back to.
    Stranded,
}

/// Whether an active buff on `entity` carries an effect `wanted` picks out.
pub fn buff_effect_applies(
    world: &World,
    entity: Entity,
    wanted: impl Fn(&EntityEffect) -> bool,
) -> bool {
    let Some(buffs) = world.entity(entity).get::<BuffsComponent>() else {
        return false;
    };
    let registry = world.resource::<ContentRegistry>();
    buffs
        .active()
        .any(|(id, _)| registry.entity_buff_def(id).effects.iter().any(&wanted))
}

/// Whether `entity` takes orders and runs none now. An entity with no order
/// queue runs none, but is not idle.
pub fn idle(world: &World, entity: Entity) -> bool {
    world
        .entity(entity)
        .get::<OrderQueueComponent>()
        .is_some_and(|queue| queue.front().is_none())
}

/// Whether `entity` bears the buff `id` now. An entity with no buffs bears
/// none.
pub fn bears(world: &World, entity: Entity, id: EntityBuffId) -> bool {
    world
        .entity(entity)
        .get::<BuffsComponent>()
        .is_some_and(|buffs| buffs.contains(id))
}

/// Whether `entity` can carry the buff `id`: it carries every stat the buff's
/// modifiers name. An effect that names no stat asks for none; an entity with
/// no stat store carries none.
pub fn can_carry(world: &World, entity: Entity, id: EntityBuffId) -> bool {
    let Some(stats) = world.entity(entity).get::<StatsComponent>() else {
        return false;
    };
    let def = world.resource::<ContentRegistry>().entity_buff_def(id);
    def.effects.iter().all(|effect| match effect {
        EntityEffect::Modifiers(modifiers) => modifiers
            .modifiers()
            .iter()
            .all(|modifier| stats.has(modifier.stat)),
        EntityEffect::Disable | EntityEffect::Conceal => true,
    })
}

/// Whether something conceals `entity` now: its type, an active buff, or a
/// field it declares a concealing effect for.
pub fn concealed(world: &World, entity: Entity) -> bool {
    let def = of(world, entity);
    match def.concealment {
        Concealment::Concealed => true,
        // Most entities are exposed by type, carry no buff and declare no
        // concealing effect: they are done in two reads.
        Concealment::Exposed => {
            buff_effect_applies(world, entity, |effect| {
                matches!(effect, EntityEffect::Conceal)
            }) || fields::concealed_of(world, def, entity)
        }
    }
}

/// Whether a change of form is under way on `entity`: it wears its origin or
/// an interim form until the change lands. Apart from its [`Operation`] — a
/// switched-off entity changes form all the same, and one changing form
/// still operates.
pub fn changing(world: &World, entity: Entity) -> bool {
    world
        .entity(entity)
        .get::<MorphComponent>()
        .is_some_and(|morph| morph.is_under_way())
}

/// Whether a change of form is under way on `entity` while it still wears the
/// form the change started from: a change with no interim form.
pub fn changing_in_origin(world: &World, entity: Entity) -> bool {
    world
        .entity(entity)
        .get::<MorphComponent>()
        .is_some_and(|morph| morph.is_under_way() && morph.from == type_id(world, entity))
}

/// Returns the [`Operation`] `entity` is in. One off the map waiting on a cell
/// is [`Operation::Stranded`] before anything else is asked; one still rising
/// is [`Operation::Rising`] whatever the fields say about it or the dock it
/// stands in.
pub fn operation(world: &World, entity: Entity) -> Operation {
    if pending_reveal(world, entity) {
        Operation::Stranded
    } else if rising(world, entity) {
        Operation::Rising
    } else if fields::disabled(world, entity) {
        Operation::Disabled(Outage::Field)
    } else if buff_effect_applies(world, entity, |effect| {
        matches!(effect, EntityEffect::Disable)
    }) {
        Operation::Disabled(Outage::Buff)
    } else if annex::idles_alone(world, entity) {
        Operation::Disabled(Outage::Orphaned)
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
        .filter(|morph| morph.is_under_way())
        .map_or_else(|| type_id(world, entity), |morph| morph.from)
}

/// What a declared quantity of ticks comes to for `entity`: a constant is
/// what it says, and a stat names the entity's effective value, rounded up.
///
/// Panics if `entity` carries no such stat — content registration checks that
/// whoever reads a quantity carries the stat it names.
pub fn quantity_ticks(world: &World, entity: Entity, quantity: Quantity) -> u32 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("simulation entity must have a stat store")
        .quantity_ticks(quantity)
        .expect("the capability pairs the entity with this stat")
}

/// What a declared quantity of cells comes to for `entity`: a constant is
/// what it says, and a stat names the entity's effective value, truncated.
///
/// Panics if `entity` carries no such stat — content registration checks that
/// whoever reads a quantity carries the stat it names.
pub fn quantity_cells(world: &World, entity: Entity, quantity: Quantity) -> u32 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("simulation entity must have a stat store")
        .quantity_cells(quantity)
        .expect("the capability pairs the entity with this stat")
}

/// What a declared quantity of whole units comes to for `entity`: a constant
/// is what it says, and a stat names the entity's effective value, truncated.
///
/// Panics if `entity` carries no such stat — content registration checks that
/// whoever reads a quantity carries the stat it names.
pub fn quantity_count(world: &World, entity: Entity, quantity: Quantity) -> u32 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("simulation entity must have a stat store")
        .quantity_count(quantity)
        .expect("the capability pairs the entity with this stat")
}

/// What `entity`, standing on the grid, marks there (see [`grid_mark_of`]):
/// on the cell model, a body mid-crossing marks the cell its crossing claimed.
///
/// Panics for an entity with no location.
pub fn grid_mark(world: &World, entity: Entity) -> Option<GridMark> {
    let def = of(world, entity);
    let mut location = world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("a body standing on the grid has a location")
        .clone();
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => {
            if movement_model::is_mid_crossing(location.position)
                && let Some(claimed) = world
                    .entity(entity)
                    .get::<MoveComponent>()
                    .and_then(|movement| movement.path.last().copied())
            {
                location.position = FixedUVec2::from(claimed);
            }
        }
        MovementModel::Continuous => {}
    }
    grid_mark_of(
        &location,
        def.location
            .expect("a body standing on the grid has a location"),
        OccupancyClass::of(def),
    )
}

/// What a body standing at `location` in `def` marks on the grid, on the plane
/// `class` selects — or `None` for one whose solidity marks no cells.
pub fn grid_mark_of(
    location: &LocationComponent,
    def: LocationDef,
    class: OccupancyClass,
) -> Option<GridMark> {
    def.solidity().claims_cells().then(|| GridMark {
        footprint: CellRect::new(body::anchor(location.position), def.size()),
        layers: def.occupies(),
        class,
    })
}

/// The shape a body of `location_def` stands and routes with.
pub fn mover_shape(registry: &ContentRegistry, location_def: &LocationDef) -> MoverShape {
    let domain = registry.domain_def(location_def.domain());
    MoverShape::new(
        domain.terrains().mask(registry),
        domain.layer(),
        location_def.occupies(),
        location_def.size(),
        domain.cliff_crossing(),
    )
}

/// Returns where `entity` stands.
///
/// Panics if `entity` is not a simulation entity, or is off the map: a hidden
/// entity stands nowhere, and [`whereabouts`] is the read for one that may be.
pub fn position(world: &World, entity: Entity) -> FixedUVec2 {
    world
        .entity(entity)
        .get::<LocationComponent>()
        .expect("an entity on the map has a location; a hidden one is read by whereabouts")
        .position
}

/// Whether `entity` is walking: a walk of its own is under way.
pub fn walking(world: &World, entity: Entity) -> bool {
    world.entity(entity).contains::<MoveComponent>()
}

/// Where `entity` is: its position while it stands on the map; off it, the
/// whereabouts of what it is [`inside`]; waiting beside a container gone from
/// the world, the footprint its return last searched; otherwise where it went
/// in.
///
/// An entity carries a location or the hidden marker, never both and never
/// neither; what it is inside may itself be inside a job.
///
/// Panics if `entity` is not a simulation entity, or its boarding names no
/// living transporter.
pub fn whereabouts(world: &World, entity: Entity) -> FixedUVec2 {
    if let Some(location) = world.entity(entity).get::<LocationComponent>() {
        return location.position;
    }
    match inside(world, entity) {
        Some(container) => whereabouts(world, container),
        None => match waiting_beside(world, entity) {
            Some((Beside::Container(_), last)) => FixedUVec2::from(last.origin),
            Some((Beside::WentIn | Beside::Footprint(_) | Beside::Berth { .. }, _)) | None => {
                went_in(world, entity)
            }
        },
    }
}

/// [`footprint`] read at `entity`'s [`whereabouts`], with the size its type
/// declares.
///
/// Panics if `entity` is not a simulation entity, or its type declares no
/// location.
pub fn whereabouts_footprint(world: &World, entity: Entity) -> (FixedUVec2, CellSize) {
    (whereabouts(world, entity), location_size(world, entity))
}

/// What `entity` is off the map inside: the transporter it rides, the job it
/// works hidden inside, or, waiting off the map for a cell, the container its
/// return is set down beside — each while it is in the world. `None` for an
/// entity on the map, one inside a link, or one inside nothing that remains.
///
/// Read from the marker's [`Inside`].
///
/// Panics if `entity` is not a simulation entity, or its boarding names no
/// living transporter.
pub fn inside(world: &World, entity: Entity) -> Option<Entity> {
    let entity_ref = world.entity(entity);
    let hidden = entity_ref.get::<HiddenComponent>()?;
    let index = world.resource::<EntityIndex>();
    match hidden.inside {
        Inside::Transporter(holder) => Some(
            index
                .alive(holder)
                .expect("a boarding names a living holder"),
        ),
        Inside::Job(job) => {
            let job = index.any(job)?;
            assert_ne!(job, entity, "nothing works hidden inside itself");
            Some(job)
        }
        Inside::Waiting { beside, .. } => match beside {
            Beside::Container(id) => index.any(id),
            Beside::WentIn | Beside::Footprint(_) | Beside::Berth { .. } => None,
        },
        // A link is no entity.
        Inside::Link(_) | Inside::Nothing => None,
    }
}

/// The cells `entity` stands on ([`standing_rect`]), or, off the map, those of
/// what it is [`inside`] out to the first that stands; once nothing it is inside
/// remains, the footprint a deferred return beside a container last searched,
/// else where it went in.
///
/// Panics if `entity` is not a simulation entity, or its type declares no
/// location.
pub fn outer_rect(world: &World, entity: Entity) -> CellRect {
    if world.entity(entity).contains::<LocationComponent>() {
        return standing_rect(world, entity);
    }
    match inside(world, entity) {
        Some(container) => outer_rect(world, container),
        None => match waiting_beside(world, entity) {
            Some((Beside::Container(_), last)) => last,
            Some((Beside::WentIn | Beside::Footprint(_) | Beside::Berth { .. }, _)) | None => {
                went_in_rect(world, entity)
            }
        },
    }
}

/// The footprint `entity` is set down beside: the one `beside` names; once
/// that is gone, the one a deferred return beside it last searched, else where
/// the entity went in — or stands, for one still on the map.
///
/// Panics if `entity` is not a simulation entity, or its type declares no
/// location.
pub fn beside_rect(world: &World, entity: Entity, beside: Beside) -> CellRect {
    match beside {
        Beside::Container(id) => match world.resource::<EntityIndex>().any(id) {
            Some(container) => outer_rect(world, container),
            None => match waiting_beside(world, entity) {
                Some((Beside::Container(waiting_id), last)) if waiting_id == id => last,
                Some((
                    Beside::Container(_)
                    | Beside::WentIn
                    | Beside::Footprint(_)
                    | Beside::Berth { .. },
                    _,
                ))
                | None => own_rect(world, entity),
            },
        },
        Beside::WentIn => own_rect(world, entity),
        Beside::Footprint(rect) => rect,
        Beside::Berth { rect, .. } => rect,
    }
}

/// The footprint a hidden `entity` had where it went in.
///
/// Panics if `entity` is not a simulation entity, is not hidden, or its type
/// declares no location.
pub fn went_in_rect(world: &World, entity: Entity) -> CellRect {
    OccupancyClass::of(of(world, entity))
        .footprint_rect(went_in(world, entity), location_size(world, entity))
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

/// The current value of `entity`'s `pool`, if it has that pool.
///
/// Panics if `entity` is not a simulation entity.
pub fn pool_value(world: &World, entity: Entity, pool: PoolId) -> Option<FixedU64> {
    world
        .entity(entity)
        .get::<PoolsComponent>()
        .expect("a simulation entity carries a pool store")
        .current(pool)
}

/// Whether `entity` has `pool`.
pub fn has_pool(world: &World, entity: Entity, pool: PoolId) -> bool {
    pool_value(world, entity, pool).is_some()
}

/// Whether some pool of `entity` follows the work of its rise.
pub fn follows_work(world: &World, entity: Entity) -> bool {
    world
        .get::<PoolsComponent>(entity)
        .expect("a simulation entity carries a pool store")
        .follows_work()
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

/// One of `entity`'s stats counted in ticks: its value rounded up.
///
/// Panics if `entity` is not a simulation entity, or carries no such stat —
/// for stats whose presence the caller's capability check already vouches for.
pub fn effective_ticks(world: &World, entity: Entity, stat: EntityStatId) -> u32 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("simulation entity must have a stat store")
        .effective_ticks(stat)
        .expect("the capability pairs the entity with this stat")
}

/// The size `entity`'s type declares, wherever it is.
///
/// Panics if `entity` is not a simulation entity, or its type declares no
/// location.
pub fn location_size(world: &World, entity: Entity) -> CellSize {
    of(world, entity)
        .location
        .expect("only entities with LocationDef have a footprint")
        .size()
}

/// Returns where `entity` stands and how much room it takes.
///
/// The two belong together — a footprint is neither without the other — and they
/// come from different places: the position is per-entity state, the size is its
/// type's. Anything measuring against an entity wants both, so it asks once.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint(world: &World, entity: Entity) -> (FixedUVec2, CellSize) {
    footprint_of(world, of(world, entity), entity)
}

/// [`footprint`] for an `entity` whose type `def` the caller already holds.
///
/// Panics if `entity` is not a simulation entity, or `def` declares no location.
pub fn footprint_of(world: &World, def: &EntityTypeDef, entity: Entity) -> (FixedUVec2, CellSize) {
    let size = def
        .location
        .expect("validated content defines a location")
        .size();
    (position(world, entity), size)
}

/// The footprint `entity` stands on as a rect of whole cells. A standing
/// form's is anchored at the cell its position rounds to ([`body::anchor`]),
/// the cells its footprint is stamped on; a mover's at the cell its position
/// floors into, the one footprint a walk can plan toward.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn footprint_rect(world: &World, entity: Entity) -> CellRect {
    footprint_rect_of(world, of(world, entity), entity)
}

/// [`footprint_rect`] for an `entity` whose type `def` the caller already
/// holds.
///
/// Panics if `entity` is not a simulation entity, or `def` declares no location.
pub fn footprint_rect_of(world: &World, def: &EntityTypeDef, entity: Entity) -> CellRect {
    let (position, size) = footprint_of(world, def, entity);
    OccupancyClass::of(def).footprint_rect(position, size)
}

/// The layer `entity`'s domain puts its body in.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn layer(world: &World, entity: Entity) -> LayerId {
    let location = of(world, entity)
        .location
        .expect("validated content stands somewhere");
    world
        .resource::<ContentRegistry>()
        .domain_def(location.domain())
        .layer()
}

/// The level `entity` stands at: the highest of its layer under the cells it
/// occupies (see [`occupied_rect`]).
///
/// Panics if `entity` is not a simulation entity, its type declares no
/// location, or a cell it occupies lies off the map — an anchor keeps a
/// footprint on the map.
pub fn level(world: &World, entity: Entity) -> u32 {
    regions::highest_level(
        world.resource::<Map>().nav_grid(),
        layer(world, entity),
        occupied_rect(world, entity),
    )
    .expect("an anchor keeps a footprint on the map")
}

/// The cells `entity`'s footprint occupies: anchored at the cell its position
/// rounds to ([`body::anchor`]), where a standing form is stamped and a
/// mover's claim lies.
///
/// Panics if `entity` is not a simulation entity, or its type declares no location.
pub fn occupied_rect(world: &World, entity: Entity) -> CellRect {
    occupied_rect_of(world, of(world, entity), entity)
}

/// [`occupied_rect`] for an `entity` whose type `def` the caller already
/// holds.
///
/// Panics if `entity` is not a simulation entity, or `def` declares no location.
pub fn occupied_rect_of(world: &World, def: &EntityTypeDef, entity: Entity) -> CellRect {
    let (position, size) = footprint_of(world, def, entity);
    CellRect::new(body::anchor(position), size)
}

/// The cells `entity` stands on, per [`body::standing_rect`] — what a reach
/// measure takes for either side, the one reaching and the one reached.
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
        Some(target) => targeting::reaches(registry, weapon.targets(), of(world, target)),
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
/// body no further than adjacency, until it is canceled.
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

/// Where a hidden `entity` went in. Panics for one on the map.
fn went_in(world: &World, entity: Entity) -> FixedUVec2 {
    world
        .entity(entity)
        .get::<HiddenComponent>()
        .expect("an entity with no location is hidden")
        .went_in
        .position
}

/// What a waiting `entity`'s return searches beside and the footprint it last
/// searched, or `None` for one on the map or not waiting.
fn waiting_beside(world: &World, entity: Entity) -> Option<(Beside, CellRect)> {
    match world.entity(entity).get::<HiddenComponent>()?.inside {
        Inside::Waiting { beside, last, .. } => Some((beside, last)),
        Inside::Transporter(_) | Inside::Job(_) | Inside::Link(_) | Inside::Nothing => None,
    }
}

/// The footprint `entity` stands on, or, off the map, the one it went in by.
fn own_rect(world: &World, entity: Entity) -> CellRect {
    if hidden(world, entity) {
        went_in_rect(world, entity)
    } else {
        footprint_rect(world, entity)
    }
}
