//! Simulation entity creation, destruction, and map presence.

use std::collections::BTreeSet;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    world::{EntityWorldMut, Mut, World},
};
use ferrets_content::{
    attack::Slain,
    base_stats::BaseStats,
    brood::{BreederDef, OrphanFate},
    dying::{DeathKind, DyingDef},
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::FieldSourceDef,
    gateway::{GatewayDef, Joins},
    location::LocationDef,
    morph::MorphReason,
    pool::PoolInitial,
    pool_def::PoolId,
    registry::ContentRegistry,
    rise::{RisePool, RisePoolInitial},
    skills::SkillId,
    transport::PassengerFate,
    work::Attachment,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use crate::{
    berths,
    beside::{Beside, Lean},
    bore_id::BoreIds,
    brood,
    components::{
        annex::{AnnexComponent, DocksComponent},
        attached::AttachedComponent,
        brood::{BredComponent, BroodComponent},
        concealed::ConcealedComponent,
        dying::{DiedComponent, DyingComponent, Passing, RemainsComponent},
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        field_source::FieldSourcesComponent,
        gateway::GatewayComponent,
        hidden::{self, HiddenComponent, Inside},
        lifetime::LifetimeComponent,
        location::LocationComponent,
        morph::MorphComponent,
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::{self, OwnerComponent},
        pool_shifts::PoolShiftsComponent,
        pools::{self, PoolsComponent},
        rally::RallyPointComponent,
        repair::UnderRepairComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent, UnderHarvestComponent},
        rise::{RiseAttendance, RiseOrigin, RisingComponent},
        stance::{Stance, StanceComponent},
        stand::{StandComponent, Standing},
        tags::TagsComponent,
        train::TrainQueueComponent,
        transport::{BoardedComponent, GarrisonFireComponent, TransporterComponent},
        tunnel::TunnelEndComponent,
        turret::{TurretState, TurretsComponent},
        warp::WarpComponent,
    },
    control_groups::ControlGroups,
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause},
    fields,
    game_loop::{
        board, buffs,
        orders::{self, Refusal},
        stats,
    },
    ground,
    impacts::PendingImpacts,
    links::{self, LinkKey, LinkPassengers, Membership},
    map::{Map, OccupancyClass},
    movement_model::{self, MovementModel},
    order::Order,
    rally,
    ruleset::RemainsLimit,
    selection::Selection,
    session::{GameSession, player_id::PlayerId},
    simulation_id::{SimulationId, SimulationIds},
    tunnels::{self, EndLoss},
};
/// Look direction a freshly spawned entity starts with: south, the conventional
/// resting facing toward the viewer.
pub(crate) const DEFAULT_FACING: Facing = Facing::SOUTH;

/// Which form a refit puts on an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wearing {
    /// A form of its own: a fresh entity's, a landed change's, or the origin
    /// a change returns to.
    Own,
    /// An interim form, worn while a change of form runs.
    Interim,
}

/// What fitting a type does about the acts it performs on standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingActs {
    /// The acts lie ahead again: the entity is coming to stand as something
    /// new.
    Rearm,
    /// Acts already performed stay performed; only a form that had none and
    /// gains some has them ahead.
    Keep,
}

/// How far an entity's field sources reach the moment it enters the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldReach {
    /// Each source starts at its declared initial reach and grows from there.
    Initial,
    /// Each source already spans its whole radius.
    Full,
}

/// How an entity enters the world.
#[derive(Debug, Clone)]
pub enum Arrival<'a> {
    /// Standing, finished: each pool at its initial, or where `starts`
    /// names it.
    Standing {
        /// The pools started in place of their type's initial.
        starts: &'a [(PoolId, PoolInitial)],
    },
    /// Rising into the world.
    Rising {
        /// How the rise is advanced, and on whose behalf.
        attendance: RiseAttendance,
        /// What brought the entity in.
        origin: RiseOrigin,
    },
}

/// Creates an entity of the given type at `position`, owned by `owner`
/// (`None` creates a neutral entity), its field sources reaching as `reach`
/// says and arriving as `arrival` says, announcing nothing.
///
/// `position` must lie exactly on a cell's origin corner — a fresh entity is
/// at rest, and rest positions are lattice points.
///
/// The `spawn_*` functions are the announcing counterparts, one per cause.
///
/// Returns `(entity, simulation_id)`, or `None` if `type_name` is not registered
/// or the position is blocked on the nav grid. Panics when a standing
/// arrival's starts name a pool the type does not declare, name one twice,
/// or start one where it cannot start, or when it arrives rising and its
/// type never rises.
pub fn create_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    arrival: Arrival,
) -> Option<(Entity, SimulationId)> {
    debug_assert!(
        !movement_model::is_mid_crossing(position),
        "entities spawn at rest: position must lie exactly on a cell origin"
    );
    // Both come off one borrow: the whole definition is never cloned for a
    // placement test that reads two of its fields.
    let (location_def, occupancy) = {
        let registry = world.resource::<ContentRegistry>();
        let def = registry.entity(type_name)?;
        match &arrival {
            Arrival::Standing { starts } => check_starts(registry, def, starts),
            Arrival::Rising { .. } => {}
        }
        (def.location?, OccupancyClass::of(def))
    };
    let location = LocationComponent::new(position, DEFAULT_FACING);
    if !ground::can_place(world, &location, location_def, occupancy, &[], &[]) {
        return None;
    }

    let (entity, id) = conjure(world, type_name, location, owner, reach, arrival)?;
    // Stamped from the location the entity now carries, not the one tested
    // above.
    restore_footprint(world, entity);
    Some((entity, id))
}

/// Creates one of the map's placements, each pool `starts` names started
/// there in place of its type's initial, and announces it as placed.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid. Panics as [`create_entity`] does.
pub fn spawn_placed(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    starts: &[(PoolId, PoolInitial)],
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Standing { starts },
        SpawnCause::Placed,
    )
}

/// Creates a unit finished in `trainer`'s queue and announces it as trained.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
pub fn spawn_trained(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    trainer: SimulationId,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Standing { starts: &[] },
        SpawnCause::Trained { trainer },
    )
}

/// Creates a unit `warper` warped in for `payer`, who owns it, rising
/// unattended over its own rise time, the warper cooling down until
/// `ready_at` for it, and announces it as warped in.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid. Panics when the type does not rise, or the
/// warper is not alive.
pub fn spawn_warped(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    payer: PlayerId,
    reach: FieldReach,
    warper: SimulationId,
    ready_at: u32,
) -> Option<(Entity, SimulationId)> {
    let warper_type = entity_def::type_id(
        world,
        world
            .resource::<EntityIndex>()
            .alive(warper)
            .expect("a warper is alive at the warp"),
    );
    spawn(
        world,
        type_name,
        position,
        Some(payer),
        reach,
        Arrival::Rising {
            attendance: RiseAttendance::Unattended { founder: warper },
            origin: RiseOrigin::Warped {
                warper_type,
                payer,
                ready_at,
            },
        },
        SpawnCause::Warped { warper },
    )
}

/// Creates a construction site `builder` placed, attended as `attendance` says,
/// and announces it as founded.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid. Panics when the type never rises.
pub fn spawn_founded(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    builder: SimulationId,
    attendance: RiseAttendance,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Rising {
            attendance,
            origin: RiseOrigin::Founded { payer: owner },
        },
        SpawnCause::Founded { builder },
    )
}

/// Creates a broodling `by` bore, seated in a berth of the breeder `job`'s
/// group named by `attachment`, and announces it as bred. The entity never
/// stands on the grid on its way to the seat.
///
/// Returns `None` if `type_name` is not registered or has no location, or the
/// group has no free berth.
pub(crate) fn spawn_bred(
    world: &mut World,
    type_name: &str,
    job: Entity,
    attachment: &Attachment,
    owner: Option<PlayerId>,
    reach: FieldReach,
    by: SimulationId,
) -> Option<(Entity, SimulationId)> {
    let from = entity_def::footprint_rect(world, job);
    let (seat, point) = berths::nearest_free_seat(world, job, attachment.berths(), from)?;
    // Facing the way every fresh entity does; the seat sets the position.
    let location = LocationComponent::new(point, DEFAULT_FACING);
    let (entity, id) = conjure(
        world,
        type_name,
        location,
        owner,
        reach,
        Arrival::Standing { starts: &[] },
    )
    .expect("validated content breeds a registered type with a location");
    seat_at(world, entity, job, attachment, seat, point);
    announce(world, id, SpawnCause::Bred { by });
    Some((entity, id))
}

/// Creates an entity handed on by the death of `of` and announces it as
/// bequeathed.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
pub fn spawn_bequeathed(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    of: SimulationId,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Standing { starts: &[] },
        SpawnCause::Bequeathed { of },
    )
}

/// Creates an entity the caster `by` raised from the remains `from`, arriving
/// as `arrival` says, and announces it as raised.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
#[allow(clippy::too_many_arguments)]
pub fn spawn_raised(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    arrival: Arrival,
    by: SimulationId,
    from: SimulationId,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        arrival,
        SpawnCause::Raised { by, from },
    )
}

/// Creates an entity the caster `by` called up from nothing, arriving as
/// `arrival` says, and announces it as summoned.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
pub fn spawn_summoned(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    arrival: Arrival,
    by: SimulationId,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        arrival,
        SpawnCause::Summoned { by },
    )
}

/// Creates the resource source put back where `by`, raised over it, died,
/// and announces it as uncovered.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
pub fn spawn_uncovered(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    by: SimulationId,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Standing { starts: &[] },
        SpawnCause::Uncovered { by },
    )
}

/// Creates an entity the sandbox spawn command conjured, and announces it as
/// such.
///
/// Returns `None` if `type_name` is not registered or the position is
/// blocked on the nav grid.
pub fn spawn_sandbox(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
) -> Option<(Entity, SimulationId)> {
    spawn(
        world,
        type_name,
        position,
        owner,
        reach,
        Arrival::Standing { starts: &[] },
        SpawnCause::Sandbox,
    )
}

/// Sets down `count` of `type_name` around the footprint at `around`, as what
/// `fallen` left behind.
///
/// The left type decides what it becomes: one tagged as remains is laid down
/// ownerless and already dying, its `lifetime` its whole existence, remembering
/// who fell there; anything else stands up as an ordinary entity of whoever
/// owned the deceased. A `slain` of [`Slain::Nothing`] leaves no body — the
/// weapon left nothing of it — and does not touch what else the death hands on.
///
/// Supply is never asked: what stands up here stands whatever the owner's
/// ceiling says, and holds its cost from then on like any other entity.
///
/// What stands up here is as new as anything trained, so its field sources
/// start at their initial reach and grow from there.
///
/// The deceased has already freed its footprint, so the search starts on the
/// ground it was standing on and works outward; what claims no cells is laid
/// down there whatever stands over it. Fewer free cells than asked for set down
/// fewer: half a brood is better than none of it.
pub(crate) fn spawn_bequest(
    world: &mut World,
    type_name: &str,
    count: u32,
    around: CellPos,
    around_size: CellSize,
    fallen: Fallen,
    slain: Slain,
) {
    let (location_def, left) = {
        let registry = world.resource::<ContentRegistry>();
        let def = registry
            .entity(type_name)
            .expect("validated content leaves registered types");
        let left = if def.is_remains() {
            Left::Remains {
                decay: def
                    .base_ticks(EntityStatId::LIFETIME)
                    .expect("a type tagged as remains carries the lifetime it lies for"),
            }
        } else {
            Left::Entity
        };
        (
            def.location.expect("a registered type stands somewhere"),
            left,
        )
    };
    match (left, slain) {
        (Left::Remains { .. }, Slain::Nothing) => return,
        (Left::Remains { .. }, Slain::Remains) | (Left::Entity, _) => {}
    }

    // The search knows what ground each of them needs: one that claims its
    // cells takes the nearest free ones, one that shares them lies where it
    // fell, under whoever is standing there. Either way it starts on the
    // ground the deceased freed a moment ago.
    let cells = world.resource::<Map>().find_placements_near(
        around,
        around_size,
        &location_def,
        count as usize,
    );

    for cell in cells {
        let position = FixedUVec2::from(cell);
        match left {
            Left::Remains { decay } => {
                spawn_remains(world, type_name, position, location_def, decay, fallen);
            }
            Left::Entity => {
                spawn_bequeathed(
                    world,
                    type_name,
                    position,
                    fallen.owner,
                    FieldReach::Initial,
                    fallen.id,
                );
            }
        }
    }
}

/// Lays one body down: an entity that never joins the alive world, is not
/// selectable or targetable but by a cast that names remains, and is removed
/// when its decay runs out.
///
/// Only the components meaningful to something lying on the ground are added:
/// identity, location, the [`RemainsComponent`] memory of who fell here, the
/// order queue with its `Die` order, and the decay. Live-gameplay components
/// from the type definition (health, movement, combat, …) are skipped — remains
/// can never use them, and a freshly initialized value (e.g. full health on a
/// body) would be false.
fn spawn_remains(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    location_def: LocationDef,
    decay: u32,
    fallen: Fallen,
) {
    debug_assert!(
        !movement_model::is_mid_crossing(position),
        "remains spawn at rest: position must lie exactly on a cell origin"
    );
    if !room_for_remains(world) {
        return;
    }
    let (type_id, class) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry
            .entity(type_name)
            .expect("the type was resolved a moment ago");
        (
            registry
                .type_id(type_name)
                .expect("the type was resolved a moment ago"),
            OccupancyClass::of(type_def),
        )
    };

    let location = LocationComponent::new(position, DEFAULT_FACING);
    // Remains that claim nothing are laid down whatever stands there: a mover
    // in the cell a body falls in shares it with the body. Remains that do
    // claim their cells — rubble — can genuinely be blocked, by what the grid
    // marks and by what has dug in under it.
    if location_def.solidity().claims_cells()
        && !ground::can_place(world, &location, location_def, class, &[], &[])
    {
        return;
    }
    world
        .resource_mut::<Map>()
        .place_entity(&location, &location_def, class);

    let id = world.resource_mut::<SimulationIds>().mint();
    let mut queue = OrderQueueComponent::default();
    queue.push(Order::Die, None);

    let entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_id, type_name),
        location,
        queue,
        PoolsComponent::default(),
        PoolShiftsComponent::default(),
        RemainsComponent {
            of: fallen.entity_type,
            owner: fallen.owner,
        },
        DyingComponent {
            ticks_remaining: decay,
            // Nothing sustains a body: what it rots into, it rots into by
            // lying there long enough.
            passing: Passing::Death {
                kind: DeathKind::Decayed,
                slain: Slain::Remains,
            },
        },
    ));
    let entity = entity_mut.id();

    world
        .resource_mut::<EntityIndex>()
        .insert_remains(id, entity);
    announce(world, id, SpawnCause::Bequeathed { of: fallen.id });
}

/// What a death hands on, once its type is resolved.
#[derive(Debug, Clone, Copy)]
enum Left {
    /// A body, lying there for the ticks its type declares.
    Remains {
        /// Ticks it lies before it is gone, its type's `lifetime`.
        decay: u32,
    },
    /// Anything else, standing up as an ordinary entity of whoever owned the
    /// deceased.
    Entity,
}

/// Who died, as what they left behind remembers them.
#[derive(Clone, Copy)]
pub(crate) struct Fallen {
    /// The entity that died here.
    pub id: SimulationId,
    /// The type it wore.
    pub entity_type: EntityTypeId,
    /// Who owned it, if anyone.
    pub owner: Option<PlayerId>,
}

/// Whether the rules leave room on the map for another body.
///
/// A death beyond the limit leaves none: what lies there already stays, and
/// decay is what makes room again.
fn room_for_remains(world: &World) -> bool {
    match world.resource::<GameSession>().rules().remains() {
        RemainsLimit::Unbounded => true,
        RemainsLimit::AtMost(limit) => {
            world.resource::<EntityIndex>().remains_count() < limit as usize
        }
    }
}

/// Takes a body off the map before its decay has run out — the counterpart of
/// [`spawn_remains`].
///
/// Its cells are freed as the end of the decay would have freed them. Nothing
/// is announced here: what takes a body announces the taking itself.
pub(crate) fn despawn_remains(world: &mut World, entity: Entity) {
    debug_assert!(
        world.entity(entity).contains::<RemainsComponent>(),
        "only remains are cleared away"
    );

    lift_footprint(world, entity);
    let id = entity_def::simulation_id(world, entity);
    world.resource_mut::<EntityIndex>().remove_dying(id);
    world.despawn(entity);
}

/// Takes an entity off the map into `inside`: frees the cells it holds, gives
/// up its berth, and takes its location, kept in [`HiddenComponent::went_in`]
/// until a return puts one back. A hidden entity holds no cells and is neither
/// selected nor targeted; [`place_back_beside`] sets it down again.
///
/// Panics for an entity already hidden: [`hidden::record_inside`] says where one
/// is now.
pub fn hide_entity(world: &mut World, entity: Entity, inside: Inside) {
    assert!(
        !entity_def::hidden(world, entity),
        "only an entity on the map is hidden"
    );
    lift_footprint(world, entity);
    unseat(world, entity);
    take_off_map(world, entity, inside);
}

/// Puts a body with no cell to stand on off the map to wait for one beside
/// `beside`, leaning as `lean` says: one still on the map, holding no cells and
/// no berth, is taken off it; one already hidden leaves what it was inside.
/// Either way it is [`Inside::Waiting`] from then on, and
/// [`game_loop::pending_reveal::process_pending_reveals`] retries the return
/// every tick until a cell opens.
fn wait_off_map(world: &mut World, entity: Entity, beside: Beside, lean: Lean) {
    let last = entity_def::beside_rect(world, entity, beside);
    let waiting = Inside::Waiting { beside, lean, last };
    if entity_def::hidden(world, entity) {
        hidden::record_inside(world, entity, waiting);
    } else {
        take_off_map(world, entity, waiting);
    }
}

/// Whether a boarding was made or refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boarding {
    /// The passenger is aboard.
    Boarded,
    /// The hold refused it, for the reason given.
    Refused(Refusal),
}

/// Puts `passenger` aboard `holder` when the hold admits it — the passenger's
/// type rides and it is neither changing form nor seating anyone, the holder
/// carries it, its owner is admitted and there is room:
/// the passenger leaves the map, joins the holder's passengers, carries the
/// boarding, and leaves every player's selection. A refused passenger is
/// untouched.
///
/// Panics unless the passenger is alive, aboard nothing, seated nowhere, on
/// the map and not the holder itself, and the holder is a transporter alive,
/// on the map and operating.
pub fn board(world: &mut World, passenger: Entity, holder: Entity) -> Boarding {
    assert!(
        !entity_def::dying(world, passenger)
            && !entity_def::boarded(world, passenger)
            && !entity_def::attached(world, passenger)
            && !entity_def::hidden(world, passenger),
        "a passenger boards alive, from the map"
    );
    assert_ne!(passenger, holder, "nothing boards itself");
    assert!(
        entity_def::of(world, holder).transporter().is_some(),
        "only a transporter takes a passenger aboard"
    );
    let holder_operation = entity_def::operation(world, holder);
    assert!(
        !entity_def::dying(world, holder)
            && !entity_def::hidden(world, holder)
            && matches!(holder_operation, Operation::Operating),
        "a passenger boards a holder alive, on the map and operating"
    );
    match board::would_board(world, passenger, holder) {
        Ok(()) => {}
        Err(refusal) => return Boarding::Refused(refusal),
    }
    put_aboard(world, passenger, holder);
    Boarding::Boarded
}

/// The boarding itself, judged by the caller.
fn put_aboard(world: &mut World, passenger: Entity, holder: Entity) {
    let passenger_id = entity_def::simulation_id(world, passenger);
    let holder_id = entity_def::simulation_id(world, holder);
    hide_entity(world, passenger, Inside::Transporter(holder_id));
    world
        .entity_mut(holder)
        .get_mut::<TransporterComponent>()
        .expect("a passenger boards a transporter")
        .passengers
        .insert(passenger_id);
    world
        .entity_mut(passenger)
        .insert(BoardedComponent { holder: holder_id });
    world.resource_mut::<Selection>().remove(passenger_id);
}

/// Takes `body` off the map into the link `key`, announcing it.
///
/// Panics for a body already off the map.
pub(crate) fn put_into_link(world: &mut World, body: Entity, key: LinkKey) {
    let body_id = entity_def::simulation_id(world, body);
    hide_entity(world, body, Inside::Link(key));
    world.resource_mut::<LinkPassengers>().insert(key, body_id);
    world.resource_mut::<Selection>().remove(body_id);
}

/// Takes an entity that holds no cells and no berth off the map into
/// `inside`, announcing it. Panics for one that is already hidden.
fn take_off_map(world: &mut World, entity: Entity, inside: Inside) {
    let announced = SimulationEvent::EntityHidden {
        entity: entity_def::simulation_id(world, entity),
        position: entity_def::position(world, entity),
    };
    world.resource_mut::<EventRecord>().emit(announced);
    mark_hidden(world, entity, inside);
}

/// Marks an entity that holds no cells and no berth as hidden inside
/// `inside`, taking its location into the marker; every shot following it
/// loses it and lands where it went in. Panics for one that is already
/// hidden: there is nothing to take.
pub(crate) fn mark_hidden(world: &mut World, entity: Entity, inside: Inside) {
    let id = entity_def::simulation_id(world, entity);
    let mut entity_mut = world.entity_mut(entity);
    let went_in = entity_mut
        .take::<LocationComponent>()
        .expect("an entity taken off the map stood somewhere");
    let position = went_in.position;
    entity_mut.insert(HiddenComponent::new(went_in, inside));
    world.resource_mut::<PendingImpacts>().lose(id, position);
}

/// Seats an entity in a berth of `job`'s group named by `attachment`: it frees
/// the cells it held, puts its middle on the free berth nearest to where it
/// stood, and holds no cells from there on while staying on the map. Bring it back onto
/// the grid with [`place_back_beside`] or [`place_back_beside_or_retry`].
///
/// Panics for a hidden entity, which has nothing to stand on, and when the
/// group has no free berth: a caller attaches only where [`berths::shut`]
/// found room.
pub(crate) fn attach_entity(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
) {
    assert!(
        !entity_def::hidden(world, entity),
        "only an entity on the map attaches to a job"
    );
    lift_footprint(world, entity);
    // A worker already seated elsewhere gives that berth up first.
    unseat(world, entity);
    seat_unseated(world, entity, job, attachment);
}

/// Seats an entity that holds no cells and no berth — one that has just given
/// its seat up through [`unseat`] — in the free berth of `job`'s group named
/// by `attachment` nearest to where it is.
///
/// Panics when the group has no free berth: a caller seats only where it
/// found room.
pub(crate) fn seat_unseated(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
) {
    let standing = entity_def::standing_rect(world, entity);
    let (seat, point) = berths::nearest_free_seat(world, job, attachment.berths(), standing)
        .expect("a worker attaches only to a job with a free berth in its group");
    seat_at(world, entity, job, attachment, seat, point);
}

/// Sits an entity off the grid down in berth `seat` of `job`'s group named by
/// `attachment`, at `point`: the berth is taken and the entity's middle put on
/// the point.
fn seat_at(
    world: &mut World,
    entity: Entity,
    job: Entity,
    attachment: &Attachment,
    seat: usize,
    point: FixedUVec2,
) {
    let id = entity_def::simulation_id(world, entity);
    let job_id = entity_def::simulation_id(world, job);
    berths::occupy(world, job, attachment.berths(), seat, id);

    let position = berths::position_centered_on(world, entity, point);
    let mut entity_mut = world.entity_mut(entity);
    entity_mut
        .get_mut::<LocationComponent>()
        .expect("only entities with LocationComponent can attach")
        .position = position;
    entity_mut.insert(AttachedComponent {
        job: job_id,
        berths: attachment.berths().to_string(),
        seat,
        stance: attachment.stance(),
        sitting: berths::sit_down(attachment.stance(), id),
        hops: 0,
    });
}

/// Takes an entity out of the berth it sits in, if any, giving the berth back
/// to the job. The entity is otherwise untouched, standing where its berth
/// was with nothing under it.
pub(crate) fn unseat(world: &mut World, entity: Entity) -> Option<AttachedComponent> {
    let attached = world.entity_mut(entity).take::<AttachedComponent>()?;
    berths::vacate(world, &attached);
    Some(attached)
}

/// Frees the footprint an entity holds on the grid. An entity already off the
/// grid — hidden, or attached to a job — holds nothing to free.
///
/// The entity is otherwise untouched, so a caller that lifts a footprint to
/// test the ground under it puts it back with [`restore_footprint`] or takes
/// the entity off the map itself.
pub(crate) fn lift_footprint(world: &mut World, entity: Entity) {
    if !entity_def::stands_on_grid(world, entity) {
        return;
    }
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef hold a footprint");
    let class = OccupancyClass::of(def);
    world.resource_scope(|world, mut map: Mut<Map>| {
        let location = world
            .get::<LocationComponent>(entity)
            .expect("only entities with LocationComponent hold a footprint");
        map.displace_entity(location, &location_def, class);
    });
}

/// Stamps back the footprint [`lift_footprint`] took off an entity that
/// otherwise stands on the grid.
pub(crate) fn restore_footprint(world: &mut World, entity: Entity) {
    if !entity_def::stands_on_grid(world, entity) {
        return;
    }
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef hold a footprint");
    let class = OccupancyClass::of(def);
    world.resource_scope(|world, mut map: Mut<Map>| {
        let location = world
            .get::<LocationComponent>(entity)
            .expect("only entities with LocationComponent hold a footprint");
        map.place_entity(location, &location_def, class);
    });
}

/// Whether a body coming back onto the map was set down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetDown {
    /// It stands on the map: set down now, or standing already.
    Placed,
    /// No free cell within the search radius; it stays off the map.
    NoCell,
}

/// Puts an entity that is off the grid back on it, on the nearest free cell:
/// a hidden one beside `beside`, the cells `lean` names first; an attached one
/// around the cell of the berth it sits at.
///
/// A hidden entity gets its location back at the chosen cell, facing the way
/// it went in; an attached one keeps its location and moves to the cell.
pub fn place_back_beside(world: &mut World, entity: Entity, beside: Beside, lean: Lean) -> SetDown {
    if entity_def::stands_on_grid(world, entity) {
        return SetDown::Placed;
    }
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can be placed back");
    match cell_back_beside(world, entity, beside, lean, &location_def) {
        Some(cell) => {
            place_back_at(world, entity, cell, &location_def);
            SetDown::Placed
        }
        None => SetDown::NoCell,
    }
}

/// Stands the hidden `entity` back on the free cell nearest `point` at most
/// `range` cells from `rect` that its placement rules admit, announcing it,
/// or `SetDown::NoCell` when none within the search radius takes it. The
/// search starts from the cell within the range nearest `point` (see
/// [`Projection::nearest_in_range_of_rect`](ferrets_geometry::projection::Projection::nearest_in_range_of_rect)).
pub(crate) fn place_back_near(
    world: &mut World,
    entity: Entity,
    point: CellPos,
    rect: CellRect,
    range: u32,
) -> SetDown {
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef can be placed back");
    let class = OccupancyClass::of(def);
    let owner = entity_def::owner(world, entity);
    let size = location_def.size();
    let spot = CellRect::new(point, CellSize::ONE);
    let map = world.resource::<Map>();
    let projection = map.projection();
    let start = projection.nearest_in_range_of_rect(rect, range, point);
    let cell = map.find_placement_near_toward_where(start, CellSize::ONE, size, spot, |cell| {
        let placed = LocationComponent::new(FixedUVec2::from(cell), DEFAULT_FACING);
        projection.in_range_for_rects(CellRect::new(cell, size), rect, range)
            && ground::can_place(world, &placed, location_def, class, &[], &[])
            && fields::allows_placement(world, owner, def, cell)
    });
    match cell {
        Some(cell) => {
            place_back_at(world, entity, cell, &location_def);
            SetDown::Placed
        }
        None => SetDown::NoCell,
    }
}

/// Stands the hidden `entity` back beside `beside` on the free cell nearest
/// it that its placement rules admit, leaning as `lean` says, announcing it,
/// or `SetDown::NoCell` when none within the search radius takes it.
pub(crate) fn place_back_beside_admitted(
    world: &mut World,
    entity: Entity,
    beside: Beside,
    lean: Lean,
) -> SetDown {
    let def = entity_def::of(world, entity);
    let location_def = def
        .location
        .expect("only entities with LocationDef can be placed back");
    let class = OccupancyClass::of(def);
    let owner = entity_def::owner(world, entity);
    let around = entity_def::beside_rect(world, entity, beside);
    let toward = match lean {
        Lean::WentIn => entity_def::went_in_rect(world, entity),
        Lean::Toward(toward) => toward,
        Lean::Nothing => around,
    };
    let cell = world.resource::<Map>().find_placement_near_toward_where(
        around.origin,
        around.size,
        location_def.size(),
        toward,
        |cell| {
            let placed = LocationComponent::new(FixedUVec2::from(cell), DEFAULT_FACING);
            ground::can_place(world, &placed, location_def, class, &[], &[])
                && fields::allows_placement(world, owner, def, cell)
        },
    );
    match cell {
        Some(cell) => {
            place_back_at(world, entity, cell, &location_def);
            SetDown::Placed
        }
        None => SetDown::NoCell,
    }
}

/// The cell [`place_back_beside`] would stand `entity` on, or `None` when
/// nothing within the search radius takes a footprint of `fits`: an attached
/// entity searches around the cell of the berth it sits at; any other beside
/// `beside`, the cells `lean` names first.
///
/// Panics for a `Lean::WentIn` on an entity that is not hidden.
pub(crate) fn cell_back_beside(
    world: &World,
    entity: Entity,
    beside: Beside,
    lean: Lean,
    fits: &LocationDef,
) -> Option<CellPos> {
    let map = world.resource::<Map>();
    if entity_def::attached(world, entity) {
        let berth = body::anchor(entity_def::position(world, entity));
        return map.find_placement_near(berth, CellSize::new(1, 1), fits);
    }
    let around = entity_def::beside_rect(world, entity, beside);
    match lean {
        Lean::WentIn => map.find_placement_near_toward(
            around.origin,
            around.size,
            fits,
            entity_def::went_in_rect(world, entity),
        ),
        Lean::Toward(toward) => {
            map.find_placement_near_toward(around.origin, around.size, fits, toward)
        }
        Lean::Nothing => map.find_placement_near(around.origin, around.size, fits),
    }
}

/// Like [`place_back_beside`], with the return deferred when no cell is free:
/// the entity waits off the map — a seated one gives its berth back and is
/// hidden — and [`game_loop::pending_reveal::process_pending_reveals`] retries
/// every tick until a cell opens.
pub(crate) fn place_back_beside_or_retry(
    world: &mut World,
    entity: Entity,
    beside: Beside,
    lean: Lean,
) {
    match place_back_beside(world, entity, beside, lean) {
        SetDown::Placed => return,
        SetDown::NoCell => {}
    }
    // A seated body gives its berth back and waits off the map like any other.
    match unseat(world, entity) {
        Some(_) => {}
        None => assert!(
            entity_def::hidden(world, entity),
            "an entity placed nowhere is seated or hidden"
        ),
    }
    wait_off_map(world, entity, beside, lean);
}

/// Stands an attached entity on `cell`, giving its berth up.
///
/// Panics for an entity that is not attached: only a seated body steps out.
pub(crate) fn step_out(world: &mut World, entity: Entity, cell: CellPos) {
    assert!(
        entity_def::attached(world, entity),
        "only an attached entity steps out of a berth"
    );
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef stand");
    place_back_at(world, entity, cell, &location_def);
}

/// Stands an entity that holds no cells and no berth — one that has just given
/// its seat up through [`unseat`] — on the nearest free cell beside `beside`,
/// leaning as `lean` says. With no free cell within the search
/// radius it is hidden instead, [`Inside::Waiting`], so
/// [`game_loop::pending_reveal::process_pending_reveals`] stands it up as soon
/// as a cell opens.
pub(crate) fn stand_or_retry(world: &mut World, entity: Entity, beside: Beside, lean: Lean) {
    let location_def = entity_def::of(world, entity)
        .location
        .expect("only entities with LocationDef can stand");
    match cell_back_beside(world, entity, beside, lean, &location_def) {
        Some(cell) => place_back_at(world, entity, cell, &location_def),
        None => wait_off_map(world, entity, beside, lean),
    }
}

/// Aligns `breeder`'s brood with the form it has just taken, `origin` being
/// the form the change was declared on. Each of the `seated` broodlings is
/// moved to a seat of the group its attachment names on the new form, nearest
/// its old berth, or **detached** when that form does not hold it — no such
/// group, or no seat left — on the breeder's orphan terms.
pub(crate) fn align_broodlings(
    world: &mut World,
    breeder: Entity,
    seated: &[SimulationId],
    origin: EntityTypeId,
) {
    let Some(fate) = brood::terms_on(world, breeder, origin).map(BreederDef::orphans) else {
        debug_assert!(
            seated.is_empty(),
            "a seated brood was bred on some form's terms"
        );
        return;
    };
    let of = entity_def::simulation_id(world, breeder);
    // Every seat is given up first, so the group is laid out afresh for the
    // form now worn before anyone sits down in it. The tie stays.
    let unseated: Vec<(Entity, CellPos)> = seated
        .iter()
        .map(|&id| {
            let entity = world
                .resource::<EntityIndex>()
                .alive(id)
                .expect("a counted broodling is alive");
            let berth = body::anchor(entity_def::position(world, entity));
            unseat(world, entity).expect("a counted broodling sits in a berth");
            (entity, berth)
        })
        .collect();
    for (entity, berth) in unseated {
        // Its own copy of the attachment: the seating below changes the world.
        let attachment = brood::broodling_attachment(world, entity)
            .cloned()
            .expect("a seated broodling's type is a broodling");
        if brood::holds(world, breeder, entity, &attachment) {
            seat_unseated(world, entity, breeder, &attachment);
        } else {
            detach_broodling(world, breeder, of, entity, berth, fate);
        }
    }
}

/// Seats a bred entity again after a change of form ended early with it back
/// in its bred form: in a free berth of its breeder's group, back on its
/// brood — or, when its breeder is gone or has no seat left for it,
/// **destroyed**. Nothing for an entity nobody bred.
pub(crate) fn reseat_broodling(world: &mut World, entity: Entity) {
    let Some(bred) = world.entity(entity).get::<BredComponent>().cloned() else {
        return;
    };
    let id = entity_def::simulation_id(world, entity);
    let Some(breeder) = world.resource::<EntityIndex>().alive(bred.by) else {
        despawn_entity(world, entity, DeathCause::Unseated { of: bred.by });
        return;
    };
    // Its own copy of the attachment: the seating below changes the world.
    let attachment = brood::broodling_attachment(world, entity)
        .cloned()
        .expect("a bred entity's type is a broodling");
    if brood::holds(world, breeder, entity, &attachment) {
        attach_entity(world, entity, breeder, &attachment);
        brood::add(world, breeder, id);
    } else {
        despawn_entity(world, entity, DeathCause::Unseated { of: bred.by });
    }
}

/// Where an entity set back down on the grid came from.
#[derive(Debug)]
enum Return {
    /// Off the map: hidden, with no location.
    OffMap,
    /// A berth it sat in until now: attached, on the map with no cells.
    Berth,
    /// On the map with no cells and no berth, its seat given up before.
    Unseated,
}

/// Puts an entity that is off the grid back on it at `cell`, announcing the
/// return of one that was hidden.
///
/// The one point every return to the grid ends at.
fn place_back_at(world: &mut World, entity: Entity, cell: CellPos, location_def: &LocationDef) {
    let class = OccupancyClass::of(entity_def::of(world, entity));
    let seat = unseat(world, entity);

    // A hidden entity comes back with the location it went in with, facing
    // the way it went in; one on the map already, seated or unseated, keeps
    // the location it has. Either is moved to the cell.
    let hidden = world.entity_mut(entity).take::<HiddenComponent>();
    let came_back = match (hidden, seat) {
        (Some(hidden), None) => {
            world.entity_mut(entity).insert(hidden.went_in);
            Return::OffMap
        }
        (None, Some(_)) => Return::Berth,
        (None, None) => Return::Unseated,
        (Some(_), Some(_)) => unreachable!("a hidden entity is never seated"),
    };
    stand_at(world, entity, cell, location_def, class);
    fit_concealment(world, entity);

    match came_back {
        Return::OffMap => {
            let announced = SimulationEvent::EntityRevealed {
                entity: entity_def::simulation_id(world, entity),
                position: entity_def::position(world, entity),
            };
            world.resource_mut::<EventRecord>().emit(announced);
        }
        Return::Berth | Return::Unseated => {}
    }
}

/// Moves entities standing on the grid to stand anchored at their cells, all
/// at once: every footprint is lifted before any is set down, so one may land
/// on cells another held. Each keeps its facing, workers seated on one follow
/// it onto its berths' new points, and each move is announced.
///
/// Panics for an entity that does not stand on the grid at rest, is dying, is
/// still rising, was raised over a resource source or is changing form: the
/// caller judged every move, and every cell free with the moved bodies lifted.
pub fn relocate(world: &mut World, moves: &[(Entity, CellPos)]) {
    let mut from = Vec::with_capacity(moves.len());
    for &(entity, _) in moves {
        assert!(
            entity_def::relocatable(world, entity),
            "only a finished body standing on the grid, over no resource source and not dying or changing form, is relocated"
        );
        let position = entity_def::position(world, entity);
        assert!(
            match world.resource::<Map>().movement_model() {
                MovementModel::Cell => !movement_model::is_mid_crossing(position),
                MovementModel::Continuous => true,
            },
            "a relocated body stands at rest, its walk dropped before it moves"
        );
        from.push(position);
    }
    for &(entity, _) in moves {
        lift_footprint(world, entity);
    }
    for (&(entity, cell), from) in moves.iter().zip(from) {
        let def = entity_def::of(world, entity);
        let class = OccupancyClass::of(def);
        let location_def = def
            .location
            .expect("only entities with LocationDef stand on the grid");
        stand_at(world, entity, cell, &location_def, class);
        fit_concealment(world, entity);
        for worker in berths::reseat(world, entity) {
            fit_concealment(world, worker);
        }
        let announced = SimulationEvent::EntityRelocated {
            entity: entity_def::simulation_id(world, entity),
            from,
            to: entity_def::position(world, entity),
        };
        world.resource_mut::<EventRecord>().emit(announced);
    }
}

/// Fits `entity` with concealment as its type, buffs and the fields where it
/// stands now say.
pub(crate) fn fit_concealment(world: &mut World, entity: Entity) {
    let concealed = entity_def::concealed(world, entity);
    fit_default::<ConcealedComponent>(&mut world.entity_mut(entity), concealed);
}

/// Moves a located entity to stand anchored at `cell` and stamps its footprint
/// there, on the plane `class` selects. Panics for an entity with no location.
fn stand_at(
    world: &mut World,
    entity: Entity,
    cell: CellPos,
    location_def: &LocationDef,
    class: OccupancyClass,
) {
    world.resource_scope(|world, mut map: Mut<Map>| {
        let mut location = world
            .get_mut::<LocationComponent>(entity)
            .expect("an entity on the map has a location");
        location.position = FixedUVec2::from(cell);
        map.place_entity(&location, location_def, class);
    });
}

/// Starts the dying phase for an alive entity.
///
/// The entity immediately leaves the alive set: it is removed from every
/// player's selection, all queued orders are force-canceled, and a `Die` order
/// is queued. The entity stays in the world as dying — still holding its
/// footprint on the nav grid under the cell model, while the continuous
/// rebuild stops counting its body — until the `Die` order completes and
/// frees it.
///
/// Takes `entity` off the map, announcing nothing — the counterpart of
/// [`create_entity`], as [`despawn_entity`] is of the `spawn_*` functions.
///
/// Nothing authored it, so it leaves nothing: no body, no brood bursting out.
/// A death the game has something to say about — what took it, and so what it
/// hands on — goes through [`despawn_entity`].
///
/// The dying phase still runs, because it is what gives back whatever the
/// entity was holding: a crew seat, a berth, the cells under it. What was in
/// its care is settled as any death settles it.
///
/// No-op if the entity is already dying or has died.
pub fn destroy_entity(world: &mut World, entity: Entity) {
    start_dying(world, entity, Passing::Removal);
}

/// Starts the dying phase, whatever the death was.
///
/// `cause` is what the entity is dying of: what its type leaves is declared
/// against the kind of death, and whether a body is left at all against the
/// weapon behind it — and by the time the dying phase ends neither is knowable
/// any more.
///
/// No-op if the entity is already dying or has died.
fn start_dying(world: &mut World, entity: Entity, passing: Passing) {
    {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<DyingComponent>() || entity_ref.contains::<DiedComponent>() {
            return;
        }
    }

    let id = entity_def::simulation_id(world, entity);
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.position);

    // The entity keeps its footprint through the dying phase, but the movement
    // state that knows which cell a crossing claimed is about to be canceled —
    // so a mid-crossing entity snaps onto its claimed cell. Continuous
    // movers die where they stand: their positions are free points and their
    // claim already tracks the cell they round to. A hidden one has no
    // crossing to finish.
    match world.resource::<Map>().movement_model() {
        MovementModel::Cell => {
            if let Some(position) = position
                && movement_model::is_mid_crossing(position)
                && let Some(claimed) = world
                    .entity(entity)
                    .get::<MoveComponent>()
                    .and_then(|mc| mc.path.last().copied())
            {
                world
                    .entity_mut(entity)
                    .get_mut::<LocationComponent>()
                    .expect("only entities with LocationComponent can be moving")
                    .position = FixedUVec2::from(claimed);
            }
        }
        MovementModel::Continuous => {}
    }

    world.resource_mut::<Selection>().remove(id);
    world.resource_mut::<ControlGroups>().remove(id);

    // What it holds, then what it is in. A ride's end leans nowhere in
    // particular; a worker comes out on the side it went in by.
    settle_passengers(world, entity, Lean::Nothing);
    settle_gateway(world, entity, GatewayLoss::Died, Lean::Nothing);
    settle_workers(world, entity, Lean::WentIn);
    settle_broodlings(world, entity, entity_def::morph_origin(world, entity));
    leave_transporter(world, entity, id);
    leave_link(world, entity, id);
    leave_brood(world, entity, id);
    // A dying entity never comes back to the grid: a waiting body waits no
    // more.
    if entity_def::pending_reveal(world, entity) {
        hidden::record_inside(world, entity, Inside::Nothing);
    }

    // What waits is the death; a type that states none goes the tick it dies,
    // and a body has already spent its whole existence lying there.
    let dying_time = entity_def::of(world, entity)
        .dying
        .as_ref()
        .and_then(DyingDef::dying_time)
        .unwrap_or(0);

    let mut entity_mut = world.entity_mut(entity);
    entity_mut.insert(DyingComponent {
        ticks_remaining: dying_time,
        passing,
    });
    if let Some(mut queue) = entity_mut.get_mut::<OrderQueueComponent>() {
        queue.push(Order::Die, Some(CancelPolicy::Force));
    }

    world.resource_mut::<EntityIndex>().mark_dying(id);
}

/// Starts the dying phase for an alive entity and announces the death.
///
/// The announcing counterpart to [`destroy_entity`]. Announced *before* the
/// teardown runs, while the entity still carries what the announcement reports;
/// anything aboard dies during that teardown, so the holder's death precedes its
/// passengers' in the record.
///
/// No-op if the entity is already dying or has died, so a death is never
/// announced twice.
pub fn despawn_entity(world: &mut World, entity: Entity, cause: DeathCause) {
    announce_death(world, entity, cause, Slain::Remains);
}

/// Starts the dying phase for an entity a weapon brought down, and announces
/// the kill.
///
/// The one death with a weapon behind it, so the one that can deny a body:
/// `slain` is what that weapon leaves of it, known now and nowhere later.
///
/// No-op if the entity is already dying or has died.
pub fn despawn_killed(
    world: &mut World,
    entity: Entity,
    by: SimulationId,
    by_owner: Option<PlayerId>,
    slain: Slain,
) {
    announce_death(world, entity, DeathCause::Killed { by, by_owner }, slain);
}

/// Announces a death and starts the phase it dies through.
fn announce_death(world: &mut World, entity: Entity, cause: DeathCause, slain: Slain) {
    {
        let entity_ref = world.entity(entity);
        if entity_ref.contains::<DyingComponent>() || entity_ref.contains::<DiedComponent>() {
            return;
        }
    }

    let announced = SimulationEvent::EntityDied {
        entity: entity_def::simulation_id(world, entity),
        entity_type: entity_def::type_id(world, entity),
        owner: entity_def::owner(world, entity),
        position: entity_def::whereabouts(world, entity),
        cause,
    };
    world.resource_mut::<EventRecord>().emit(announced);

    start_dying(
        world,
        entity,
        Passing::Death {
            kind: cause.into(),
            slain,
        },
    );
}

/// Hands `entity` to `to`, announcing the capture, with `by` naming what took
/// it; a passenger the new owner no longer admits and that is set down leans
/// as `lean` says.
///
/// The entity keeps its health and its pools. Its orders are dropped and what
/// they took stays spent, and it leaves every player's selection and control
/// groups, the two stores that hold ids.
///
/// Panics if `to` already owns it.
pub fn change_owner(world: &mut World, entity: Entity, to: PlayerId, by: SimulationId, lean: Lean) {
    let from = entity_def::owner(world, entity);
    assert!(
        from != Some(to),
        "an entity is handed only to a player that does not already own it"
    );

    let id = entity_def::simulation_id(world, entity);
    // Whatever it was doing was the old owner's business: the orders go, and
    // what they took stays spent — losing the entity loses the work in it.
    // Dropping them can start a death, on a change of form whose type declares
    // one when interrupted, and what died under the old owner is not captured.
    orders::drop_all(world, entity, CancelPolicy::Force);
    match world.resource::<EntityIndex>().alive(id) {
        Some(_) => {}
        None => return,
    }

    world.resource_mut::<Selection>().remove(id);
    world.resource_mut::<ControlGroups>().remove(id);

    // Announced before what it costs, as a death is: the capture precedes the
    // passengers it loses in the record.
    let announced = SimulationEvent::EntityCaptured {
        entity: id,
        entity_type: entity_def::type_id(world, entity),
        from,
        to,
        by,
        position: entity_def::whereabouts(world, entity),
    };
    world.resource_mut::<EventRecord>().emit(announced);

    // What changing hands costs what it holds: the link it leaves, judged
    // against the old owner's gateways, and the passengers `to` does not
    // admit.
    settle_gateway(world, entity, GatewayLoss::Captured { to }, lean);
    settle_unadmitted_passengers(world, entity, to, lean);

    // A rally set on an entity still rising was the old owner's choice of
    // where it goes, or where what it releases goes.
    if entity_def::rising(world, entity)
        && let Some(mut rally) = world.get_mut::<RallyPointComponent>(entity)
    {
        rally.0 = None;
    }
    world.entity_mut(entity).insert(OwnerComponent::new(to));
}

/// Keeps aboard a transporter handed to `to` only what `to`'s boarding admits;
/// every other passenger follows the transporter's passenger fate, in id
/// order, one set down leaning as `lean` says. Nothing for an entity that
/// carries nobody.
fn settle_unadmitted_passengers(world: &mut World, entity: Entity, to: PlayerId, lean: Lean) {
    let Some((boarding, fate)) = entity_def::of(world, entity)
        .transporter()
        .map(|transporter| (transporter.boarding(), transporter.passenger_fate()))
    else {
        return;
    };
    let passengers = world
        .entity(entity)
        .get::<TransporterComponent>()
        .expect("a transporter type carries a passenger list")
        .passengers
        .clone();
    if passengers.is_empty() {
        return;
    }
    let holder = entity_def::simulation_id(world, entity);

    for id in passengers {
        let Some(passenger) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        let admitted = owner::admits(
            world.resource::<GameSession>(),
            boarding,
            Some(to),
            entity_def::owner(world, passenger),
        );
        if admitted {
            continue;
        }
        world
            .entity_mut(entity)
            .get_mut::<TransporterComponent>()
            .expect("a transporter type carries a passenger list")
            .passengers
            .remove(&id);
        settle_passenger(
            world,
            passenger,
            holder,
            fate,
            DeathCause::PassengerBarred { holder },
            lean,
        );
    }
}

/// Keeps the link of `entity`, a gateway changing into the form `into`, when
/// `into` joins what it joins, and leaves it as a loss does otherwise — into
/// a gateway of anything else or into no gateway — set down leaning as `lean`
/// says. Nothing for an entity that is no gateway.
pub(crate) fn keep_or_leave_link(
    world: &mut World,
    entity: Entity,
    into: EntityTypeId,
    lean: Lean,
) {
    let Some(joins) = entity_def::of(world, entity)
        .gateway()
        .map(GatewayDef::joins)
    else {
        return;
    };
    let joins_into = world
        .resource::<ContentRegistry>()
        .def(into)
        .gateway()
        .map(GatewayDef::joins);
    if joins_into == Some(joins) {
        return;
    }
    settle_gateway(world, entity, GatewayLoss::Changed, lean);
}

/// How a gateway was lost to its link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayLoss {
    /// It started dying.
    Died,
    /// It changed hands, to this player.
    Captured {
        /// Its new owner.
        to: PlayerId,
    },
    /// It changed into a form that is no gateway of its link.
    Changed,
}

/// Settles what losing `gateway` as `loss` says costs: a tunnel's other end
/// fares as the tunnel says, a link no other gateway keeps any longer meets
/// its fate beside `gateway`, one set down leaning as `lean` says, and a
/// tunnel end keeps what the loss leaves it of its state. Nothing for an
/// entity that is no gateway, nor for a network gateway a capture leaves in
/// the link it was one of.
fn settle_gateway(world: &mut World, gateway: Entity, loss: GatewayLoss, lean: Lean) {
    let Some(joins) = entity_def::of(world, gateway)
        .gateway()
        .map(GatewayDef::joins)
    else {
        return;
    };
    // The link it leaves, read before anything below changes it.
    let membership = links::membership_of(world, gateway);
    match (joins, loss) {
        (Joins::Network(_), GatewayLoss::Captured { to }) => {
            let after = links::membership_from(
                world.resource::<ContentRegistry>(),
                world.resource::<GameSession>(),
                entity_def::of(world, gateway),
                Some(to),
                None,
            );
            if after == membership {
                return;
            }
        }
        (Joins::Network(_), GatewayLoss::Died | GatewayLoss::Changed) | (Joins::Tunnel(_), _) => {}
    }
    // A razed other end starts dying first, so it keeps the link no longer.
    match (joins, loss) {
        (Joins::Tunnel(_), GatewayLoss::Captured { .. }) => {
            tunnels::end_lost(world, gateway, EndLoss::Captured)
        }
        (Joins::Tunnel(_), GatewayLoss::Died | GatewayLoss::Changed) => {
            tunnels::end_lost(world, gateway, EndLoss::Gone)
        }
        (Joins::Network(_), _) => {}
    }
    match membership {
        Membership::Joined(key) => {
            if !links::kept_by_others(world, key, gateway) {
                // A link lost with its gateway's death is lost to whatever took
                // the gateway down; one a capture or a change of form left is
                // a loss nobody is credited with.
                let holder = entity_def::simulation_id(world, gateway);
                let lost = match loss {
                    GatewayLoss::Died => DeathCause::PassengerLost { holder },
                    GatewayLoss::Captured { .. } | GatewayLoss::Changed => {
                        DeathCause::PassengerBarred { holder }
                    }
                };
                settle_link(world, gateway, key, lost, lean);
            }
        }
        Membership::Apart => {}
    }
    match joins {
        Joins::Tunnel(_) => tunnel_end_after(world, gateway, loss),
        Joins::Network(_) => {}
    }
}

/// What the tunnel end `end` keeps of its state after `loss`: a dying end
/// all of it, a captured one starts on a newly minted bore, and a changed one
/// sheds it for its new form's to be fitted afresh.
fn tunnel_end_after(world: &mut World, end: Entity, loss: GatewayLoss) {
    match loss {
        GatewayLoss::Died => {}
        GatewayLoss::Captured { .. } => tunnels::bore_anew(world, end),
        GatewayLoss::Changed => {
            world.entity_mut(end).remove::<TunnelEndComponent>();
        }
    }
}

/// Applies the fate of the link `gateway` is one of to everyone inside
/// `key`, in id order, as a dying transporter's to its passengers: each is
/// let go of the link and taken to have been at `gateway`, where it dies of
/// `lost` or is set down beside. Nothing for a link that holds nobody.
fn settle_link(world: &mut World, gateway: Entity, key: LinkKey, lost: DeathCause, lean: Lean) {
    let Some(fate) = links::fate_of(world, gateway) else {
        return;
    };
    let holder = entity_def::simulation_id(world, gateway);
    let at = entity_def::whereabouts(world, gateway);
    let passengers = world.resource_mut::<LinkPassengers>().take(key);
    for id in passengers {
        let Some(passenger) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        hidden::record_inside(world, passenger, Inside::Nothing);
        let facing = world
            .get::<HiddenComponent>(passenger)
            .expect("a body in a link is hidden")
            .went_in
            .facing;
        hidden::record_went_in(world, passenger, LocationComponent::new(at, facing));
        settle_passenger(world, passenger, holder, fate, lost, lean);
    }
}

/// Applies a dying transporter's declared passenger fate to everyone aboard.
///
/// Ejection is one placement attempt per passenger, in id order, on the cells
/// its placement rules admit; a passenger no such cell takes dies with its
/// holder.
fn settle_passengers(world: &mut World, entity: Entity, lean: Lean) {
    let Some(passengers) = world
        .entity_mut(entity)
        .get_mut::<TransporterComponent>()
        .map(|mut transporter| std::mem::take(&mut transporter.passengers))
    else {
        return;
    };
    if passengers.is_empty() {
        return;
    }
    let fate = entity_def::of(world, entity)
        .transporter()
        .expect("a passenger list belongs to a transporter")
        .passenger_fate();
    let holder = entity_def::simulation_id(world, entity);

    for id in passengers {
        let Some(passenger) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        settle_passenger(
            world,
            passenger,
            holder,
            fate,
            DeathCause::PassengerLost { holder },
            lean,
        );
    }
}

/// Sets down beside a dying `job` every worker hidden inside it, leaning as
/// `lean` says; one with no cell waits off the map for one.
fn settle_workers(world: &mut World, job: Entity, lean: Lean) {
    let job_ref = world.entity(job);
    let crew: BTreeSet<SimulationId> = job_ref
        .get::<UnderHarvestComponent>()
        .map(|harvest| harvest.carriers.clone())
        .into_iter()
        .chain(
            job_ref
                .get::<UnderRepairComponent>()
                .map(|repair| repair.repairers.clone()),
        )
        .chain(
            job_ref
                .get::<RisingComponent>()
                .and_then(|rising| match &rising.attendance {
                    RiseAttendance::Crew { workers } => Some(workers.clone()),
                    RiseAttendance::Unattended { .. } | RiseAttendance::Halted => None,
                }),
        )
        .flatten()
        .collect();
    let job_id = entity_def::simulation_id(world, job);
    for id in crew {
        let Some(worker) = world.resource::<EntityIndex>().alive(id) else {
            continue;
        };
        // A hidden crew member works inside the job; one in the open or in a
        // berth leaves through its own order.
        if entity_def::hidden(world, worker) {
            place_back_beside_or_retry(world, worker, Beside::Container(job_id), lean);
        }
    }
}

/// Settles one passenger of `holder` on `fate`: destroyed with `lost` as the
/// cause, or set down on the nearest free cell beside `holder` that its
/// placement rules admit, leaning as `lean` says, and destroyed with `lost`
/// when there is none. Its boarding ends after it is settled.
fn settle_passenger(
    world: &mut World,
    passenger: Entity,
    holder: SimulationId,
    fate: PassengerFate,
    lost: DeathCause,
    lean: Lean,
) {
    match fate {
        PassengerFate::Destroy => despawn_entity(world, passenger, lost),
        PassengerFate::Eject => {
            match place_back_beside_admitted(world, passenger, Beside::Container(holder), lean) {
                SetDown::Placed => {}
                SetDown::NoCell => despawn_entity(world, passenger, lost),
            }
        }
    }
    world
        .entity_mut(passenger)
        .remove::<(BoardedComponent, GarrisonFireComponent)>();
}

/// Settles every broodling `breeder` counts on the orphan terms it breeds on
/// with `origin` as the form its change was declared on, taking them off its
/// brood: perishing ones die in id order, lingering ones are set down by the
/// berths they sat at, the tie cut either way. Nothing for a breeder that
/// breeds nothing on either form.
pub(crate) fn settle_broodlings(world: &mut World, breeder: Entity, origin: EntityTypeId) {
    let Some(fate) = brood::terms_on(world, breeder, origin).map(BreederDef::orphans) else {
        return;
    };
    let of = entity_def::simulation_id(world, breeder);
    let Some(broodlings) = world
        .entity_mut(breeder)
        .get_mut::<BroodComponent>()
        .map(|mut brood| std::mem::take(&mut brood.broodlings))
    else {
        return;
    };

    for id in broodlings {
        let broodling = world
            .resource::<EntityIndex>()
            .alive(id)
            .expect("a counted broodling is alive");
        world.entity_mut(broodling).remove::<BredComponent>();
        match fate {
            OrphanFate::Perish => despawn_entity(world, broodling, DeathCause::Orphaned { of }),
            OrphanFate::Linger(_) => {
                // Set down by the berth it sat at, where it was seen.
                let berth = body::anchor(entity_def::position(world, broodling));
                let size = entity_def::footprint(world, broodling).1;
                unseat(world, broodling);
                stand_or_retry(
                    world,
                    broodling,
                    Beside::Footprint(CellRect::new(berth, size)),
                    Lean::Nothing,
                );
            }
        }
    }
}

/// Detaches a broodling that has given its seat up and that the form
/// `breeder` wears does not hold, on `fate`'s terms: perishing, it dies
/// **unseated**; lingering, its tie is cut and it is set down by `berth`, the
/// cell it sat at.
fn detach_broodling(
    world: &mut World,
    breeder: Entity,
    of: SimulationId,
    entity: Entity,
    berth: CellPos,
    fate: OrphanFate,
) {
    match fate {
        OrphanFate::Perish => despawn_entity(world, entity, DeathCause::Unseated { of }),
        OrphanFate::Linger(_) => {
            brood::untie(world, breeder, entity);
            let size = entity_def::footprint(world, entity).1;
            stand_or_retry(
                world,
                entity,
                Beside::Footprint(CellRect::new(berth, size)),
                Lean::Nothing,
            );
        }
    }
}

/// Takes a dying broodling off its breeder's count.
fn leave_brood(world: &mut World, entity: Entity, id: SimulationId) {
    let Some(bred) = world.entity_mut(entity).take::<BredComponent>() else {
        return;
    };
    if let Some(breeder) = world.resource::<EntityIndex>().alive(bred.by) {
        brood::remove(world, breeder, id);
    }
}

/// Takes a dying passenger off its transporter's list, freeing the slots it
/// held, and takes it to have been where the transporter is.
fn leave_transporter(world: &mut World, entity: Entity, id: SimulationId) {
    let Some(boarded) = world.entity_mut(entity).take::<BoardedComponent>() else {
        return;
    };
    let holder = world
        .resource::<EntityIndex>()
        .alive(boarded.holder)
        .expect("a boarding names a living holder");
    world
        .entity_mut(holder)
        .get_mut::<TransporterComponent>()
        .expect("a holder with something aboard is a transporter")
        .passengers
        .remove(&id);
    hidden::record_inside(world, entity, Inside::Nothing);
    let at = entity_def::whereabouts(world, holder);
    let facing = world
        .get::<HiddenComponent>(entity)
        .expect("a body aboard is hidden")
        .went_in
        .facing;
    hidden::record_went_in(world, entity, LocationComponent::new(at, facing));
}

/// Takes a dying passenger off the link it is inside. Nothing for an entity
/// inside no link.
fn leave_link(world: &mut World, entity: Entity, id: SimulationId) {
    let key = match world
        .get::<HiddenComponent>(entity)
        .map(|hidden| &hidden.inside)
    {
        Some(Inside::Link(key)) => *key,
        Some(
            Inside::Transporter(_) | Inside::Job(_) | Inside::Waiting { .. } | Inside::Nothing,
        )
        | None => return,
    };
    world.resource_mut::<LinkPassengers>().remove(key, id);
}

/// Removes a dead entity from the world after its dying phase has completed.
///
/// Panics if the entity has not finished dying.
pub fn remove_dead_entity(world: &mut World, entity: Entity) {
    debug_assert!(
        world.entity(entity).contains::<DiedComponent>(),
        "remove_dead_entity requires a finished dying phase"
    );

    let id = entity_def::simulation_id(world, entity);

    world.resource_mut::<EntityIndex>().remove_dying(id);
    world.despawn(entity);
}

/// Seeds `entity`'s stat store from a type's base stats — built-in and custom
/// alike, its pools' included. Buffs later fold these into `effective` (see
/// [`game_loop::stats::recompute_entity_stats`](crate::game_loop::stats::recompute_entity_stats)).
///
/// Replaces the whole store, because bases belong to the type: a type change
/// must not leave a stat the old type carried and the new one does not.
pub(crate) fn seed_stats(world: &mut World, entity: Entity, base_stats: &BaseStats) {
    let mut stats = StatsComponent::default();
    for (stat, value) in base_stats.iter() {
        stats.set_base(stat, value);
    }
    world.entity_mut(entity).insert(stats);
}

/// Fits `entity`'s components to what `type_id` requires: inserts the ones the
/// type needs and removes the ones it no longer does.
///
/// **Live state on a component both types need is left alone.** A holder keeps
/// its passengers through a type change, a trainer its queue, a carrier its
/// load — inserting a fresh default would silently empty them, which is the
/// quietest way this could go wrong.
///
/// Stance is preserved when present, because a player sets it deliberately;
/// only an entity that has none is given its type's default. Field sources
/// reach as far as `reach` says, the acts on standing are armed or kept as
/// `acts` says, and what a change of form carries through an interim form —
/// a brood, a rally point — stays or goes as `wearing` says.
pub(crate) fn fit_components(
    world: &mut World,
    entity: Entity,
    type_id: EntityTypeId,
    reach: FieldReach,
    acts: StandingActs,
    wearing: Wearing,
) {
    let (
        can_attack,
        mounted_turrets,
        can_move,
        has_health,
        trainer,
        warper,
        transporter,
        gateway,
        source,
        carrier,
        annex,
        docks,
        tags,
        skills,
        field_sources,
        acts_on_standing,
        breeds,
        releases_units,
        timed_life,
    ) = {
        let registry = world.resource::<ContentRegistry>();
        let def = registry.def(type_id);
        (
            def.can_attack(),
            def.turrets.len(),
            def.can_move(),
            def.has_pool(PoolId::HEALTH),
            def.trainer.is_some(),
            def.warper.is_some(),
            def.can_transport(),
            def.gateway().map(GatewayDef::joins),
            def.resource_source.is_some(),
            def.resource_carrier.is_some(),
            def.annex.is_some(),
            !def.docks.is_empty(),
            def.tags.clone(),
            def.skills.clone(),
            def.field_sources.clone(),
            !def.on_stand.is_empty(),
            def.breeder.is_some(),
            rally::releases_units(registry, def),
            def.base_stat(EntityStatId::LIFETIME).is_some(),
        )
    };
    // Whether the entity stands concealed is read against the type, the buffs
    // and the fields as they are now, by the same reads the tick's refit
    // makes.
    let concealed = entity_def::concealed(world, entity);
    // A rally point serves whatever releases units, the form making one by a
    // change of form now, and a mover still rising, for where it goes once it
    // has risen.
    let producing = match wearing {
        Wearing::Interim => world
            .entity(entity)
            .get::<MorphComponent>()
            .and_then(|morph| entity_def::morph_terms(world, morph))
            .is_some_and(|transition| match transition.reason() {
                MorphReason::Production => true,
                MorphReason::Change => false,
            }),
        Wearing::Own => false,
    };
    let rising_mover = can_move && world.entity(entity).contains::<RisingComponent>();
    let wants_rally = releases_units || producing || rising_mover;
    let mut entity_mut = world.entity_mut(entity);

    fit_stance(&mut entity_mut, can_attack, can_move && has_health);

    // Runtime-state components for the roles the type carries; the
    // type-constant config stays on the definition, read via its handle.
    //
    // Losing a role drops its state where it stands, which is right for
    // everything except a queue whose entries were paid for up front. The
    // order lifecycle empties that queue before a type change reaches here —
    // a flushed Train order clears it, and what its entries cost stays spent —
    // and debug builds hold the lifecycle to it.
    debug_assert!(
        trainer
            || entity_mut
                .get::<TrainQueueComponent>()
                .is_none_or(|queue| queue.0.is_empty()),
        "a type change must not drop a paid production queue"
    );
    // How long an instance has stood is live state, so a change of form that
    // keeps a timed life keeps the age with it: what is summoned for forty
    // seconds has forty seconds whatever it turns into.
    fit_default::<LifetimeComponent>(&mut entity_mut, timed_life);
    fit_default::<ConcealedComponent>(&mut entity_mut, concealed);
    fit_default::<TrainQueueComponent>(&mut entity_mut, trainer);
    fit_default::<WarpComponent>(&mut entity_mut, warper);
    fit_default::<TransporterComponent>(&mut entity_mut, transporter);
    fit_default::<GatewayComponent>(&mut entity_mut, gateway.is_some());
    // An end is born unbound on a bore of its own; the birth that raised it
    // binds it after.
    fit_with(
        &mut entity_mut,
        gateway.and_then(Joins::tunnel).is_some(),
        |entity_mut| {
            let bore = entity_mut.world_scope(|world| world.resource_mut::<BoreIds>().mint());
            TunnelEndComponent::unbound(bore)
        },
    );
    fit_default::<ResourceSourceComponent>(&mut entity_mut, source);
    fit_default::<ResourceCarrierComponent>(&mut entity_mut, carrier);
    // Which primary an annex stands with is re-derived every tick, so a form
    // that becomes an annex starts alone and is docked by the next pass.
    fit_with(&mut entity_mut, annex, |_| AnnexComponent::alone());
    // A rally point is live state — where the player pointed — and a brood
    // too — the broodlings counted, the ticks toward the next — so each stays
    // through an interim form that releases or breeds nothing on the way.
    fit_kept_while_interim::<RallyPointComponent>(&mut entity_mut, wants_rally, wearing);
    fit_kept_while_interim::<BroodComponent>(&mut entity_mut, breeds, wearing);
    // Offering docks is a standing fact about a type, so the component that
    // records what stands in them is fitted with the form: a primary with
    // every dock empty still has one, and a form that stops offering docks
    // drops what stood in them there and then.
    fit_default::<DocksComponent>(&mut entity_mut, docks);
    fit_turrets(&mut entity_mut, mounted_turrets);
    fit_vocabulary(&mut entity_mut, tags, skills);
    fit_standing_acts(&mut entity_mut, acts_on_standing, acts);
    fit_field_sources(&mut entity_mut, &field_sources, reach);
}

/// Inserts a default `C` when the type wants one and it is absent, removes it
/// when the type does not, and leaves an existing one untouched — so whatever
/// live state it holds survives.
fn fit_default<C: Component + Default>(entity: &mut EntityWorldMut, wanted: bool) {
    fit_with(entity, wanted, |_| C::default());
}

/// Inserts the `C` that `make` builds when the type wants one and it is
/// absent, removes it when the type does not, and leaves an existing one
/// untouched.
fn fit_with<C: Component>(
    entity: &mut EntityWorldMut,
    wanted: bool,
    make: impl FnOnce(&mut EntityWorldMut) -> C,
) {
    match (wanted, entity.contains::<C>()) {
        (true, false) => {
            let component = make(entity);
            entity.insert(component);
        }
        (false, true) => {
            entity.remove::<C>();
        }
        (true, true) | (false, false) => {}
    }
}

/// Fits a default `C` as [`fit_default`] does, except that a form `wearing`
/// an interim form keeps one it no longer wants.
fn fit_kept_while_interim<C: Component + Default>(
    entity: &mut EntityWorldMut,
    wanted: bool,
    wearing: Wearing,
) {
    match (wanted, entity.contains::<C>(), wearing) {
        (true, false, _) => {
            entity.insert(C::default());
        }
        (false, true, Wearing::Own) => {
            entity.remove::<C>();
        }
        (false, true, Wearing::Interim) | (true, true, _) | (false, false, _) => {}
    }
}

/// Gives an entity with no stance yet its default one: an armed one defends
/// itself, an unarmed one that `flees` (it moves and can be hurt) flees, and
/// the rest have no initiative to configure.
fn fit_stance(entity: &mut EntityWorldMut, can_attack: bool, flees: bool) {
    if entity.contains::<StanceComponent>() {
        return;
    }
    if can_attack {
        entity.insert(StanceComponent(Stance::Defend));
    } else if flees {
        entity.insert(StanceComponent(Stance::Flee));
    }
}

/// Fits `mounted` turrets: mounted looking the way the body does, and left
/// where each was last trained through any change that keeps its mount — a
/// form with more guns than the last trains the new ones forward, and one
/// with fewer drops the guns it no longer has.
fn fit_turrets(entity: &mut EntityWorldMut, mounted: usize) {
    if mounted == 0 {
        entity.remove::<TurretsComponent>();
        return;
    }
    let facing = entity
        .get::<LocationComponent>()
        .expect("a placed entity has a location")
        .facing;
    let mut turrets = entity.take::<TurretsComponent>().unwrap_or_default();
    turrets.0.resize(mounted, TurretState::mounted(facing));
    entity.insert(turrets);
}

/// Replaces the entity's tags and skills outright with the type's own.
fn fit_vocabulary(entity: &mut EntityWorldMut, tags: BTreeSet<String>, skills: Vec<SkillId>) {
    if tags.is_empty() {
        entity.remove::<TagsComponent>();
    } else {
        entity.insert(TagsComponent::new(tags));
    }
    if skills.is_empty() {
        entity.remove::<SkillsComponent>();
    } else {
        entity.insert(SkillsComponent::new(skills));
    }
}

/// Arms the acts a type performs on standing, as `acts` says, for a type
/// that has any.
fn fit_standing_acts(entity: &mut EntityWorldMut, acts_on_standing: bool, acts: StandingActs) {
    if !acts_on_standing {
        entity.remove::<StandComponent>();
        return;
    }
    match acts {
        StandingActs::Rearm => {
            entity.insert(StandComponent(Standing::Pending));
        }
        StandingActs::Keep => {
            if !entity.contains::<StandComponent>() {
                entity.insert(StandComponent(Standing::Pending));
            }
        }
    }
}

/// Fits the type's field sources, reaching as far as `reach` asks: left to
/// grow, a source takes over the reach of the one it replaces at the same
/// index.
fn fit_field_sources(
    entity: &mut EntityWorldMut,
    field_sources: &[FieldSourceDef],
    reach: FieldReach,
) {
    if field_sources.is_empty() {
        entity.remove::<FieldSourcesComponent>();
        return;
    }
    let sources = match reach {
        FieldReach::Full => FieldSourcesComponent::full(field_sources),
        FieldReach::Initial => match entity.get::<FieldSourcesComponent>() {
            Some(previous) => FieldSourcesComponent::carried(field_sources, previous),
            None => FieldSourcesComponent::seeded(field_sources),
        },
    };
    entity.insert(sources);
}

/// Brings an entity of the given type into the world at `location`, owned by
/// `owner` and arriving as `arrival` says, with everything its type gives an
/// instance — and no footprint on the grid: the caller stands it on the grid
/// or seats it. Returns `None` if `type_name` is not registered or has no
/// location. Panics when it arrives rising and its type never rises.
fn conjure(
    world: &mut World,
    type_name: &str,
    location: LocationComponent,
    owner: Option<PlayerId>,
    reach: FieldReach,
    arrival: Arrival,
) -> Option<(Entity, SimulationId)> {
    // Only what standing the entity needs; every capability component is fitted
    // from the type by `fit_components` below.
    let (type_id, base_stats) = {
        let registry = world.resource::<ContentRegistry>();
        let type_id = registry.type_id(type_name)?;
        let type_def = registry.entity(type_name)?;
        type_def.location?;
        match (&arrival, &type_def.rise) {
            (Arrival::Rising { .. }, None) => {
                panic!("a rising entity is of a type that rises: '{type_name}'")
            }
            (Arrival::Rising { .. }, Some(_)) | (Arrival::Standing { .. }, _) => {}
        }
        (type_id, type_def.base_stats.clone())
    };

    let id = world.resource_mut::<SimulationIds>().mint();

    let mut entity_mut = world.spawn((
        EntityInfoComponent::new(id, type_id, type_name),
        location,
        OrderQueueComponent::default(),
        PoolsComponent::default(),
        PoolShiftsComponent::default(),
    ));
    if let Some(player) = owner {
        entity_mut.insert(OwnerComponent::new(player));
    }
    // A rising entity rises from the moment it is spawned, before anything is
    // fitted on it.
    match &arrival {
        Arrival::Rising { attendance, origin } => {
            entity_mut.insert(RisingComponent::new(attendance.clone(), *origin));
        }
        Arrival::Standing { .. } => {}
    }
    let entity = entity_mut.id();

    seed_stats(world, entity, &base_stats);
    stats::recompute_stats_of(world, entity);
    // Current-value pools, seeded at their initial against their effective
    // max stats. A change of form carries them instead, which is why seeding
    // them is the spawn's own business.
    seed_pools(world, entity, type_id, &arrival, Seeding::First);
    fit_components(
        world,
        entity,
        type_id,
        reach,
        StandingActs::Rearm,
        Wearing::Own,
    );
    // The type's passives are fitted on the pools just seeded, and the pools
    // seeded again under the maxima the passives raise or lower, so a fresh
    // entity starts at its initial whatever its passives do to a pool.
    buffs::refit_entity(world, entity);
    stats::recompute_stats_of(world, entity);
    seed_pools(world, entity, type_id, &arrival, Seeding::AfterPassives);
    brood::open(world, entity);
    world.resource_mut::<EntityIndex>().insert_alive(id, entity);

    Some((entity, id))
}

/// Creates an entity as [`create_entity`] does and announces it with
/// `cause`.
fn spawn(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    reach: FieldReach,
    arrival: Arrival,
    cause: SpawnCause,
) -> Option<(Entity, SimulationId)> {
    let (entity, id) = create_entity(world, type_name, position, owner, reach, arrival)?;
    tunnels::bind_at_birth(world, entity, cause);
    announce(world, id, cause);
    Some((entity, id))
}

/// Announces that the entity `id` entered the world, by `cause`.
fn announce(world: &mut World, id: SimulationId, cause: SpawnCause) {
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::EntitySpawned { entity: id, cause });
}

/// Starts each pool the type `type_id` declares on `entity` as `arrival`
/// says, against its effective maximum: a standing entity's at its initial,
/// or where the arrival's starts name it; a rising entity's as its type's
/// rise holds it, a pool gained on risen left off. The first pass gives the
/// entity its pools; the pass after the passives starts them again under the
/// maxima the passives leave.
fn seed_pools(
    world: &mut World,
    entity: Entity,
    type_id: EntityTypeId,
    arrival: &Arrival,
    seeding: Seeding,
) {
    let def = world.resource::<ContentRegistry>().def(type_id);
    let starts: Vec<(PoolId, Start)> = def
        .base_stats
        .pools()
        .map(|pool| {
            let start = match arrival {
                Arrival::Standing { starts } => Start::At(
                    starts
                        .iter()
                        .find_map(|(named, initial)| (*named == pool.id()).then_some(*initial))
                        .unwrap_or(pool.initial()),
                ),
                Arrival::Rising { .. } => match def
                    .rise
                    .as_ref()
                    .and_then(|rise| rise.pool(pool.id()))
                    .expect("a rising entity's type says how it holds each pool")
                {
                    RisePool::Initial => Start::At(pool.initial()),
                    RisePool::FollowsWork(initial) => Start::FollowsWork(initial),
                    RisePool::GainedOnRisen => Start::GainedOnRisen,
                },
            };
            (pool.id(), start)
        })
        .collect();
    for (pool, start) in starts {
        match (start, seeding) {
            (Start::At(initial), Seeding::First) => pools::seed(world, entity, pool, initial),
            (Start::At(initial), Seeding::AfterPassives) => {
                pools::reseed(world, entity, pool, initial);
            }
            (Start::FollowsWork(initial), Seeding::First) => {
                pools::seed_following_work(world, entity, pool, initial.into());
            }
            (Start::FollowsWork(initial), Seeding::AfterPassives) => {
                pools::reseed_following_work(world, entity, pool, initial.into());
            }
            (Start::GainedOnRisen, Seeding::First | Seeding::AfterPassives) => {}
        }
    }
}

/// Which of a spawn's two pool passes runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seeding {
    /// The first, giving the fresh entity its pools.
    First,
    /// The one after the passives, starting the pools again under the maxima
    /// they leave.
    AfterPassives,
}

/// Where a spawn's pool passes start one pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// At this initial, following the pool's maximum.
    At(PoolInitial),
    /// At this initial, following a rising entity's work.
    FollowsWork(RisePoolInitial),
    /// Not at all: a rising entity gains the pool once it has risen.
    GainedOnRisen,
}

/// Checks each pool `starts` names for an entity of `def`: one its type
/// declares, named once, at an initial the pool can start at.
fn check_starts(registry: &ContentRegistry, def: &EntityTypeDef, starts: &[(PoolId, PoolInitial)]) {
    for (index, (pool, initial)) in starts.iter().enumerate() {
        let name = registry.pool_name(*pool);
        let declared = def.base_stats.pool(*pool).unwrap_or_else(|| {
            panic!(
                "'{}' starts the {name} pool, which its type does not declare",
                def.name
            )
        });
        assert!(
            starts[..index].iter().all(|(earlier, _)| earlier != pool),
            "'{}' starts the {name} pool twice",
            def.name
        );
        declared.validate_initial(*initial, &format!("'{}' starts the {name} pool", def.name));
    }
}
