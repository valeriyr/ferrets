//! Build order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeSet;

use bevy_ecs::{
    entity::Entity,
    query::{With, Without},
    world::World,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_nav_grid::layer_mask::LayerMask;

use super::{
    chase::{self, Destination},
    crew::{self, Departure},
    orders::{self, Processing, Refusal},
    rise::{self, Worked},
    work,
};
use crate::{
    annex, berths,
    beside::Beside,
    components::{
        build::{BuildComponent, BuildStage},
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        hidden::{self, Inside},
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        patience::Waiting,
        rise::{RiseAttendance, RisingComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SpendCause},
    fields, ground,
    landing::{self, Bound, ComesFrom, Lander},
    make_way::{self, Clearance},
    occupancy::{GridMark, OccupancyClass},
    order::{BuildTask, Order},
    overbuild::{self, Cover},
    reach, requirements,
    resources::{self, PlayerResources},
    session::player_id::PlayerId,
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
    supply, tunnels,
};
use ferrets_content::{
    build::BuilderAttendance, entity_stats::EntityStatId, registry::ContentRegistry,
    work::WorkPresence,
};

/// Whether `entity` may start this Build. To found a site: its type raises
/// the ordered type, that type rises, it operates, attending the site would
/// not take it off the map from under others seated on it, and, for a builder
/// that never walks, the site lies within its build range and the builder
/// does not stand in its way; whether the ground admits the site is decided
/// on arrival. To join one: the site still rises, its type is one the builder
/// raises, the builder operates and is free to attend, and one that never
/// walks reaches it.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    match order.build_task().expect("called with a Build order") {
        BuildTask::Found {
            type_name,
            position,
        } => can_found(world, entity, type_name, *position),
        BuildTask::Join { site } => can_join(world, entity, *site).map(|_| ()),
    }
}

/// Called once when a Build order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    let build_component = match order.build_task().expect("called with a Build order") {
        BuildTask::Found { .. } => BuildComponent::founding(),
        BuildTask::Join { .. } => BuildComponent::joining(),
    };
    world.entity_mut(entity).insert(build_component);
    OrderState::InProcessing
}

/// Called when a Build order resumes from `Suspended` (its walk to the site just
/// finished). The driver component survives suspension; validation happens in
/// [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Build entry that has a cancel policy.
///
/// Construction stops immediately under both policies. The site's fate is the
/// last builder's to decide as it leaves — torn down, or left standing halted
/// — so pulling one worker off a shared site leaves the rest to finish it.
/// Nothing is paid back: only a cancel aimed at the site refunds it.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    entry_state: OrderState,
    world: &mut World,
) -> Processing {
    // A queued entry was never prepared: the driver on the entity, if any,
    // belongs to the build under way in front of it.
    match entry_state {
        OrderState::New => return Processing::state(OrderState::Finished),
        OrderState::InProcessing | OrderState::Suspended => {}
        OrderState::Finished => unreachable!("Finished entries never stay in the queue"),
    }
    let Some(build_component) = world.entity_mut(entity).take::<BuildComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    match build_component.stage {
        // Nothing is taken up yet: there is no site to leave.
        BuildStage::Founding(_) | BuildStage::Joining => {}
        BuildStage::Raising(building_id) => leave_site(world, entity, building_id),
    }

    Processing::state(OrderState::Finished)
}

/// Whether a Build can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance a Build order by one tick.
///
/// Until the site is taken up: walk to within the builder's `build_range` of it
/// (suspending on a chase move), then either join the site under way — the one a
/// joining order names, or a matching one standing where a founding order goes —
/// or pay the price and place one, a builder in the open in the site's way first
/// walking off it, and own and allied movers on the ground asked off and waited
/// for (see [`make_way::wait_off`]). The order finishes early if the ground refuses
/// the site or the wait runs out, if a joined site is gone, has risen or is dying, if
/// the site is already held by a builder that will not share it, if the price cannot
/// be paid, if a builder in the site's way cannot walk, or finds no cell off it or no
/// way there, or if others sat down on the builder since the order started and its
/// attendance could take it off the map from under them.
///
/// After that: every builder on the site advances the same progress counter by one
/// tick's work. When the rise time is reached the rising marker is removed,
/// and a builder that raised the site from inside comes back out beside it. A
/// site not the builder's side's is left: the builder steps off it, and the
/// last one out decides its fate (see [`leave_site`]).
///
/// A builder that leaves the site unattended is done the moment the site
/// stands: the site advances itself from there (see
/// [`rise::advance_unattended`]). One consumed by its work is despawned
/// as the site completes instead of stepping back out. A site whose crew left
/// it halted is taken up again by the next builder sent to it.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let task = order
        .build_task()
        .expect("called with a Build order")
        .clone();
    let Some(build_component) = world.entity_mut(entity).take::<BuildComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    match (build_component.stage, task) {
        (BuildStage::Raising(site), _) => construct(world, entity, build_component, site),
        (
            BuildStage::Founding(_),
            BuildTask::Found {
                type_name,
                position,
            },
        ) => found(world, entity, build_component, &type_name, position),
        (BuildStage::Joining, BuildTask::Join { site }) => {
            join(world, entity, build_component, site)
        }
        (BuildStage::Founding(_), BuildTask::Join { .. })
        | (BuildStage::Joining, BuildTask::Found { .. }) => {
            unreachable!("a Build's stage follows its task")
        }
    }
}

//
// ─── Founding, joining and building ─────────────────────────────────────────
//

/// One tick of founding a site of `type_name` at `position`: walk within build
/// range of it, then join a matching own site already under way there, or pay
/// the price and raise one — a builder in the open in the site's way first
/// walking off it, own and allied movers on the ground asked off and waited
/// for (see [`make_way::wait_off`]).
fn found(
    world: &mut World,
    entity: Entity,
    mut build_component: BuildComponent,
    type_name: &str,
    position: FixedUVec2,
) -> Processing {
    let (building_location_def, price) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name).expect("type checked in prepare");
        (
            type_def
                .location
                .expect("validated content defines a location"),
            type_def.price.clone(),
        )
    };
    let site_anchor = CellPos::from(position);
    let size = building_location_def.size();

    // A builder in the open in the site's way already reaches it, and is
    // walked off it only once the site is judged.
    let site = CellRect::new(site_anchor, size);
    let in_way = in_the_way(world, entity, type_name, position);
    match in_way {
        InTheWay::Clear => {
            match chase::advance_to_point(
                &mut build_component.last_chase,
                world,
                entity,
                position,
                size,
                entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE),
            ) {
                Destination::OutOfReach => return Processing::state(OrderState::Finished),
                // A builder that never walks gives the order up as soon
                // as the site lies beyond its reach.
                Destination::Walk(move_order) => {
                    if !entity_def::of(world, entity).can_move() {
                        return Processing::state(OrderState::Finished);
                    }
                    world.entity_mut(entity).insert(build_component);
                    return Processing::suspend(move_order);
                }
                Destination::Arrived => {}
            }
        }
        InTheWay::InItsWay => {}
    }

    chase::face(world, entity, position, size);

    let owner = entity_def::owner(world, entity);

    // Work already under way here is joined rather than started again: the site
    // holds the cells, so a second placement could only ever fail.
    if let Some(under_way) = site_under_way_at(world, type_name, site_anchor, owner) {
        return take_up(world, entity, build_component, under_way);
    }

    // Every gate is judged before anything moves.
    if let Some(player) = owner {
        let def = world
            .resource::<ContentRegistry>()
            .entity(type_name)
            .expect("type checked in prepare");
        if !supply::allows(world, player, def) {
            return Processing::state(OrderState::Finished);
        }
        // Requirements gate the placement: a site already standing keeps
        // its crew even when its requirement falls.
        if !requirements::met(world, player, Some(entity), &def.requires) {
            return Processing::state(OrderState::Finished);
        }
        if !world
            .resource::<PlayerResources>()
            .can_afford(player, &price)
        {
            return Processing::state(OrderState::Finished);
        }
    }
    // Fields and docks gate the placement the same way: judged at the
    // raise, never again for a site already standing.
    let overbuilds = {
        let def = world
            .resource::<ContentRegistry>()
            .entity(type_name)
            .expect("type checked in prepare");
        if !fields::allows_placement(world, owner, def, site_anchor)
            || !annex::allows_placement(world, entity, def, site_anchor)
        {
            return Processing::state(OrderState::Finished);
        }
        def.overbuilds.clone()
    };

    // A site raised over a resource source needs one of the type under it,
    // on exactly its footprint; the source's cells then count as the
    // site's own.
    let cover = match overbuilds {
        Some(over) => {
            let Some(source) = overbuild::source_under(world, &over, site) else {
                return Processing::state(OrderState::Finished);
            };
            Cover::Source(source)
        }
        None => Cover::Nothing,
    };

    // The ground takes the site with the source off it and the builder in
    // nobody's way: it steps off, steps inside or takes a berth. The cells
    // it holds are set aside while it stands on the grid. Own and allied
    // movers standing there are asked off, and the founding waits for them
    // a while.
    let builder_mark: Vec<GridMark> = mark_on_grid(world, entity).into_iter().collect();
    let clearance = make_way::clearance(
        world,
        owner,
        site,
        building_location_def.occupies(),
        &[entity],
        |marks, bodies| {
            let lifted: Vec<GridMark> = builder_mark.iter().chain(marks).copied().collect();
            let ignored: Vec<Entity> = std::iter::once(entity)
                .chain(bodies.iter().copied())
                .collect();
            ground_takes_site(world, type_name, position, cover, &lifted, &ignored)
        },
    );
    match clearance {
        Clearance::Clear => {}
        Clearance::Clears(bodies) => {
            let patience = match &mut build_component.stage {
                BuildStage::Founding(patience) => patience,
                BuildStage::Joining | BuildStage::Raising(_) => {
                    unreachable!("a site is founded only before one is raised")
                }
            };
            let bodies: Vec<(Entity, CellRect)> =
                bodies.into_iter().map(|body| (body, site)).collect();
            return match make_way::wait_off(world, owner, &bodies, patience) {
                Waiting::GivesUp => Processing::state(OrderState::Finished),
                Waiting::Waits => {
                    world.entity_mut(entity).insert(build_component);
                    Processing::state(OrderState::InProcessing)
                }
            };
        }
        Clearance::Held => return Processing::state(OrderState::Finished),
    }

    // A builder others sat down on since the order started gives the
    // order up before raising anything.
    match requires_free_to_attend(world, entity) {
        Ok(()) => {}
        Err(_) => return Processing::state(OrderState::Finished),
    }

    // A builder in the open in the site's way walks onto a cell off the
    // site from which it still reaches it, and the site is judged again
    // once it stands there; one that never walks, or finds no such cell
    // or no way there, gives the order up.
    match in_way {
        InTheWay::InItsWay => {
            if !entity_def::of(world, entity).can_move() {
                return Processing::state(OrderState::Finished);
            }
            let Some(off) = cell_off_site(world, entity, site) else {
                return Processing::state(OrderState::Finished);
            };
            return match chase::advance_onto_point(
                &mut build_component.last_chase,
                world,
                entity,
                FixedUVec2::from(off),
            ) {
                Destination::OutOfReach => Processing::state(OrderState::Finished),
                Destination::Walk(move_order) => {
                    world.entity_mut(entity).insert(build_component);
                    Processing::suspend(move_order)
                }
                Destination::Arrived => {
                    unreachable!("a builder standing on a cell off the site is out of its way")
                }
            };
        }
        InTheWay::Clear => {}
    }

    // A builder that disappears into its work leaves the map now, and one
    // that attaches to the site leaves the grid, either freeing any of
    // the site's cells it stood on; the attached one sits in its berths
    // once the site stands. The site is judged for them as the judge
    // above set them aside: the cells they held may still read as claimed
    // until the claims are rebuilt. The site is not raised yet, so the
    // builder is inside nothing until it takes the site up.
    let (lifted, ignored): (&[GridMark], Vec<Entity>) = match attendance(world, entity) {
        BuilderAttendance::Crew(WorkPresence::Hidden { .. }) | BuilderAttendance::Consumed => {
            spawn::hide_entity(world, entity, Inside::Nothing);
            (&builder_mark, Vec::new())
        }
        BuilderAttendance::Crew(WorkPresence::Attached(_)) => {
            spawn::free_to_seat(world, entity);
            (&builder_mark, vec![entity])
        }
        BuilderAttendance::Crew(WorkPresence::Present { .. }) | BuilderAttendance::Unattended => {
            (&[], Vec::new())
        }
    };

    let builder = entity_def::simulation_id(world, entity);
    let rise_attendance = founding_attendance(world, entity, builder);
    // The source comes off the map so the site can take its cells.
    overbuild::lift(world, cover);
    let (building, building_sim_id) = spawn::spawn_founded(
        world,
        type_name,
        position,
        owner,
        FieldReach::Initial,
        builder,
        rise_attendance.clone(),
        lifted,
        &ignored,
    )
    .unwrap_or_else(|| unreachable!("the ground was judged to take the site this tick"));
    overbuild::settle(world, cover, Some(building));

    // A builder hidden before the site was raised is inside it now; one
    // off the grid sits down in the site's berths; one in the open takes
    // it up where it stands, and one that leaves it to itself stays where
    // it stands.
    match attendance(world, entity) {
        BuilderAttendance::Crew(WorkPresence::Hidden { .. }) | BuilderAttendance::Consumed => {
            hidden::record_inside(world, entity, Inside::Job(building_sim_id));
        }
        BuilderAttendance::Crew(WorkPresence::Attached(attachment)) => {
            spawn::seat_unseated(world, entity, building, &attachment);
        }
        BuilderAttendance::Crew(WorkPresence::Present { .. }) | BuilderAttendance::Unattended => {}
    }
    if let Some(player) = owner {
        resources::charge(
            world,
            player,
            price,
            SpendCause::Construction {
                site: building_sim_id,
            },
        );
    }

    match rise_attendance {
        RiseAttendance::Crew { .. } => {}
        RiseAttendance::Unattended { .. } => {
            return Processing::state(OrderState::Finished);
        }
        RiseAttendance::Halted => unreachable!("a site is never raised halted"),
    }
    build_component.stage = BuildStage::Raising(building_sim_id);
    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// One tick of joining the own site `site` under way: walk within build range
/// of it, then take it up. The order ends when the site is gone, has risen or
/// is dying — a builder sent to join a site never founds one.
fn join(
    world: &mut World,
    entity: Entity,
    mut build_component: BuildComponent,
    site: SimulationId,
) -> Processing {
    let Ok(building) = can_join(world, entity, site) else {
        return Processing::state(OrderState::Finished);
    };
    match chase::advance_to_entity(
        &mut build_component.last_chase,
        world,
        entity,
        building,
        entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE),
    ) {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        // A builder that never walks gives the order up as soon as the site
        // lies beyond its reach.
        Destination::Walk(move_order) => {
            if !entity_def::of(world, entity).can_move() {
                return Processing::state(OrderState::Finished);
            }
            world.entity_mut(entity).insert(build_component);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }
    chase::face_entity(world, entity, building);
    take_up(world, entity, build_component, site)
}

/// Takes up `site`, standing within reach of it: the builder joins its crew,
/// or the order ends when the site shuts it out, others sat down on the
/// builder since the order started, or it leaves the site to itself. Nothing
/// is placed and nothing is paid for.
fn take_up(
    world: &mut World,
    entity: Entity,
    mut build_component: BuildComponent,
    site: SimulationId,
) -> Processing {
    if site_excludes(world, site, entity) {
        return Processing::state(OrderState::Finished);
    }
    match requires_free_to_attend(world, entity) {
        Ok(()) => {}
        Err(_) => return Processing::state(OrderState::Finished),
    }
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("a site is taken up only when found standing this tick");
    let taken = join_site(world, site, entity);
    enter_site(world, entity, building);
    // A builder that leaves the site to itself is done the moment it has taken
    // it up, exactly as it is done the moment it raises one.
    match taken {
        Taken::OnCrew => {}
        Taken::LeftToItself => return Processing::state(OrderState::Finished),
    }
    build_component.stage = BuildStage::Raising(site);
    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// One tick of `entity`'s work on the site `building_id` it has taken up:
/// every builder on the site advances the same progress by one tick's work.
fn construct(
    world: &mut World,
    entity: Entity,
    build_component: BuildComponent,
    building_id: SimulationId,
) -> Processing {
    // Construction phase: one tick of this builder's work, whoever else is on the
    // site. Every way out of it leaves the site; the consumed builder's way out
    // is its death.
    let Some(building) = world.resource::<EntityIndex>().alive(building_id) else {
        // The building was destroyed mid-construction.
        work::leave(world, entity, Beside::Container(building_id));
        return Processing::state(OrderState::Finished);
    };

    // Another builder on the same site may have finished it first.
    if !entity_def::rising(world, building) {
        return leave_finished_site(world, entity, building_id);
    }
    // A site not the builder's side's is not its to raise.
    if !entity_def::same_owner(world, entity, building) {
        leave_site(world, entity, building_id);
        return Processing::state(OrderState::Finished);
    }
    let rise_time = entity_def::of(world, building)
        .rise_time()
        .expect("a rising site's type rises");
    let builder = entity_def::simulation_id(world, entity);
    match rise::put_in_work(world, building, rise_time, builder) {
        Worked::Risen => return leave_finished_site(world, entity, building_id),
        Worked::Going => {}
    }

    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// Whether `entity` may found a site of `type_name` at `position` (see
/// [`can_start`]).
fn can_found(
    world: &World,
    entity: Entity,
    type_name: &str,
    position: FixedUVec2,
) -> Result<(), Refusal> {
    if !entity_def::of(world, entity)
        .builder
        .as_ref()
        .is_some_and(|builder_def| builder_def.can_build(type_name))
    {
        return Err(Refusal::Incapable);
    }
    let Some(raised) = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .filter(|def| def.rise.is_some())
    else {
        return Err(Refusal::Incapable);
    };
    // A bound tunnel end has its other end already.
    if tunnels::refuses_raising(world, entity, raised) {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    requires_free_to_attend(world, entity)?;
    if entity_def::of(world, entity).can_move() {
        return Ok(());
    }
    if !reaches_site(world, entity, type_name, position) {
        return Err(Refusal::OutOfReach);
    }
    match in_the_way(world, entity, type_name, position) {
        InTheWay::InItsWay => Err(Refusal::NoRoom),
        InTheWay::Clear => Ok(()),
    }
}

/// The site `site` that `entity` may join, or why it may not (see
/// [`can_start`]): an own site still rising, not dying, of a type the builder
/// raises.
fn can_join(world: &World, entity: Entity, site: SimulationId) -> Result<Entity, Refusal> {
    let Some(building) = world.resource::<EntityIndex>().interactable(world, site) else {
        return Err(Refusal::TargetGone);
    };
    if !entity_def::rising(world, building)
        || entity_def::dying(world, building)
        || !entity_def::same_owner(world, entity, building)
    {
        return Err(Refusal::TargetUnfit);
    }
    let raised = &entity_def::of(world, building).name;
    if !entity_def::of(world, entity)
        .builder
        .as_ref()
        .is_some_and(|builder_def| builder_def.can_build(raised))
    {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    requires_free_to_attend(world, entity)?;
    if !entity_def::of(world, entity).can_move() {
        let range = entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE);
        if !reach::in_reach(
            world,
            entity,
            entity_def::standing_rect(world, building),
            range,
        ) {
            return Err(Refusal::OutOfReach);
        }
    }
    Ok(building)
}

/// Removes the unfinished `building` from the map, paying nothing back.
pub(super) fn tear_down_site(world: &mut World, building: Entity) {
    spawn::despawn_entity(world, building, DeathCause::Canceled);
}

/// Destroys an unfinished site, called by the last builder to walk away from a
/// site it does not leave standing. What the site cost stays
/// spent: only a cancel aimed at the site pays it back.
///
/// Panics for a site that is gone or has risen.
fn abandon_site(world: &mut World, site: SimulationId) {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("the last builder leaves a site that still stands");
    assert!(
        entity_def::rising(world, building),
        "the last builder leaves a site still rising"
    );
    tear_down_site(world, building);
}

/// Leaves the unfinished `site` standing without a crew, its progress kept,
/// called by the last builder to walk away from a site it leaves standing.
fn halt_site(world: &mut World, site: SimulationId) {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("the last builder leaves a site that still stands");
    let mut building_mut = world.entity_mut(building);
    let mut marker = building_mut
        .get_mut::<RisingComponent>()
        .expect("the last builder leaves a site still rising");
    match &marker.attendance {
        RiseAttendance::Crew { workers } => {
            debug_assert!(workers.is_empty(), "the last builder out leaves no crew");
            marker.attendance = RiseAttendance::Halted;
        }
        RiseAttendance::Unattended { .. } | RiseAttendance::Halted => {
            unreachable!("only a crewed site is left halted")
        }
    }
}

/// The unfinished site of `type_name` standing at `origin` for `owner`, if there is
/// one.
///
/// A site that has started dying is not one: joining it would mean attaching to
/// something already on its way off the map.
fn site_under_way_at(
    world: &mut World,
    type_name: &str,
    origin: CellPos,
    owner: Option<PlayerId>,
) -> Option<SimulationId> {
    let mut query = world.query_filtered::<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&OwnerComponent>,
    ), (With<RisingComponent>, Without<DyingComponent>)>();

    // The lowest id wins rather than whichever the query happens to visit first:
    // a footprint that claims no cells can sit on top of another, so more than one
    // site can answer to a cell, and query order is not something to settle a
    // shared outcome on.
    query
        .iter(world)
        .filter(|(info, location, site_owner)| {
            info.type_name() == type_name
                && CellPos::from(location.position) == origin
                && site_owner.map(|o| o.player()) == owner
        })
        .map(|(info, _, _)| info.id())
        .min()
}

/// Whether a builder stands in the way of the site it is to raise.
#[derive(Debug, Clone, Copy)]
enum InTheWay {
    /// Nothing the builder does keeps the site off the ground: it leaves the
    /// grid as the site goes up, stands off the site's cells or layers, or
    /// the site is refused or taken whether it stands there or not.
    Clear,
    /// It stays in the open while it works, and the ground refuses the site
    /// with it standing there and takes it with the builder set aside.
    InItsWay,
}

/// Whether `entity`, a builder, stands in the way of a site of `type_name` at
/// `position`: one staying in the open while it works, standing on a cell of
/// the site on a layer the site takes, where the ground — any source under
/// the site set aside — refuses the site with the builder counted and takes
/// it with the builder set aside.
fn in_the_way(world: &World, entity: Entity, type_name: &str, position: FixedUVec2) -> InTheWay {
    match attendance(world, entity) {
        BuilderAttendance::Crew(WorkPresence::Hidden { .. } | WorkPresence::Attached(_))
        | BuilderAttendance::Consumed => return InTheWay::Clear,
        BuilderAttendance::Crew(WorkPresence::Present { .. }) | BuilderAttendance::Unattended => {}
    }
    let def = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("the ordered type is registered");
    let site_def = def.location.expect("validated content defines a location");
    let site = CellRect::new(CellPos::from(position), site_def.size());
    let builder_def = entity_def::of(world, entity)
        .location
        .expect("a builder stands somewhere");
    if builder_def.occupies() & site_def.occupies() == LayerMask::EMPTY
        || !entity_def::standing_rect(world, entity).intersects(site)
    {
        return InTheWay::Clear;
    }
    // A site over a source is judged with the source off it; one with no
    // source under it is refused by the founding itself.
    let cover = match def.overbuilds.as_deref() {
        Some(over) => match overbuild::source_under(world, over, site) {
            Some(source) => Cover::Source(source),
            None => return InTheWay::Clear,
        },
        None => Cover::Nothing,
    };
    if ground_takes_site(world, type_name, position, cover, &[], &[]) {
        return InTheWay::Clear;
    }
    let builder_mark: Vec<GridMark> = mark_on_grid(world, entity).into_iter().collect();
    if ground_takes_site(world, type_name, position, cover, &builder_mark, &[entity]) {
        InTheWay::InItsWay
    } else {
        InTheWay::Clear
    }
}

/// Whether the ground takes a site of `type_name` at `position` with
/// `cover`'s source, the `marks` and the `bodies` set aside.
fn ground_takes_site(
    world: &World,
    type_name: &str,
    position: FixedUVec2,
    cover: Cover,
    marks: &[GridMark],
    bodies: &[Entity],
) -> bool {
    let def = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("the ordered type is registered");
    let site_def = def.location.expect("validated content defines a location");
    let (mut lifted, mut ignored) = overbuild::set_aside(world, cover);
    lifted.extend_from_slice(marks);
    ignored.extend_from_slice(bodies);
    let placed = LocationComponent::new(position, spawn::DEFAULT_FACING);
    ground::can_place(
        world,
        &placed,
        site_def,
        OccupancyClass::of(def),
        &lifted,
        &ignored,
    )
}

/// Whether `entity` reaches a site of `type_name` at `position` within its
/// build range from where it stands.
fn reaches_site(world: &World, entity: Entity, type_name: &str, position: FixedUVec2) -> bool {
    let site_def = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("the ordered type is registered")
        .location
        .expect("validated content defines a location");
    let site = CellRect::new(CellPos::from(position), site_def.size());
    let range = entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE);
    reach::in_reach(world, entity, site, range)
}

/// The cells `entity` marks on the grid, while it stands on it.
fn mark_on_grid(world: &World, entity: Entity) -> Option<GridMark> {
    if entity_def::stands_on_grid(world, entity) {
        entity_def::grid_mark(world, entity)
    } else {
        None
    }
}

/// The anchor nearest `entity` it can walk to off `site`, from which it still
/// reaches the site within its build range, or `None` when none will do.
fn cell_off_site(world: &World, entity: Entity, site: CellRect) -> Option<CellPos> {
    let range = entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE);
    let around = entity_def::standing_rect(world, entity);
    let lifted: Vec<_> = mark_on_grid(world, entity).into_iter().collect();
    let lander = Lander {
        def: entity_def::of(world, entity),
        owner: entity_def::owner(world, entity),
        from: ComesFrom::of(world, entity),
        bound: Bound::Range { rect: site, range },
        around,
        toward: around,
    };
    landing::walk_goal(world, lander, &lifted, |_, footprint| {
        !footprint.intersects(site) && reach::reaches(world, entity, footprint, site, range)
    })
}

/// Whether `entity` is shut out of `site`: by the crew already on it, because
/// the site takes no crew, or for want of a berth to sit in. A halted site
/// shuts nobody out by its crew.
fn site_excludes(world: &World, site: SimulationId, entity: Entity) -> bool {
    let Some(building) = world.resource::<EntityIndex>().alive(site) else {
        return false;
    };
    let Some(marker) = world.entity(building).get::<RisingComponent>() else {
        return false;
    };
    let crewed_out = match &marker.attendance {
        RiseAttendance::Halted => false,
        RiseAttendance::Crew { .. } | RiseAttendance::Unattended { .. } => {
            crew::excludes::<RisingComponent>(world, building, entity, |world, builder| {
                attendance(world, builder).crewing()
            })
        }
    };
    crewed_out
        || attendance(world, entity)
            .presence()
            .is_some_and(|presence| berths::shut(world, building, &presence))
}

/// Whether a builder that has taken a site up stays with it.
enum Taken {
    /// It is on the site's crew, and its order carries on.
    OnCrew,
    /// It left the site to itself, and its order is done.
    LeftToItself,
}

/// Takes up `site`: joins the crew on it, or becomes the crew of a halted one.
/// The site's rising marker was put on with the first builder; a newcomer
/// joins what is there and marks nothing.
fn join_site(world: &mut World, site: SimulationId, entity: Entity) -> Taken {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("a site is taken up only when found standing this tick");
    let rise_attendance = world
        .entity(building)
        .get::<RisingComponent>()
        .expect("a site is taken up only when found under way this tick")
        .attendance
        .clone();
    match rise_attendance {
        RiseAttendance::Crew { .. } => {
            crew::join_existing::<RisingComponent>(world, building, entity);
            Taken::OnCrew
        }
        // A halted site is taken up on the taker's own terms, the same terms it
        // would have been raised on: a builder that stays joins its crew, and
        // one that leaves a site to itself leaves this one to itself too.
        RiseAttendance::Halted => {
            let builder = entity_def::simulation_id(world, entity);
            let attendance = founding_attendance(world, entity, builder);
            let taken = match attendance {
                RiseAttendance::Crew { .. } => Taken::OnCrew,
                RiseAttendance::Unattended { .. } => Taken::LeftToItself,
                RiseAttendance::Halted => unreachable!("a site taken up is attended"),
            };
            world
                .entity_mut(building)
                .get_mut::<RisingComponent>()
                .expect("checked above")
                .attendance = attendance;
            taken
        }
        RiseAttendance::Unattended { .. } => {
            unreachable!("an unattended site shuts every builder out")
        }
    }
}

/// What a site becomes when a builder of `entity`'s attendance raises or takes
/// it up: a crew of one for a builder that stays with it, and unattended for
/// one that leaves it to itself.
fn founding_attendance(world: &World, entity: Entity, builder: SimulationId) -> RiseAttendance {
    match attendance(world, entity) {
        BuilderAttendance::Crew(_) | BuilderAttendance::Consumed => RiseAttendance::Crew {
            workers: BTreeSet::from([builder]),
        },
        BuilderAttendance::Unattended => RiseAttendance::Unattended { founder: builder },
    }
}

/// Drops out of the crew on `site`, saying whether it was the last one out.
///
/// The marker stays behind either way: it carries the work raised so far, which is not
/// the crew's to take away. A site that finished or was destroyed in the meantime is
/// gone as a job and is nobody's to tear down.
fn leave_crew(world: &mut World, site: SimulationId, entity: Entity) -> Departure {
    match world.resource::<EntityIndex>().alive(site) {
        Some(building) => crew::leave::<RisingComponent>(world, building, entity),
        None => Departure::JobGone,
    }
}

/// Puts the builder where its attendance has it stand as it takes up `site`:
/// a crew builder as its presence says, one consumed by the site hidden inside
/// it. One that leaves the site unattended stays exactly where it stands.
fn enter_site(world: &mut World, entity: Entity, site: Entity) {
    if let Some(presence) = attendance(world, entity).presence() {
        work::enter(world, entity, &presence, site);
    }
}

/// Takes `entity` off the crew of the rising `site` before the work is done:
/// it steps out beside the site as it stands, and the last one out decides
/// the site's fate by how it attended it: one that stood on or beside the
/// site leaves it standing for whoever takes it up next; one that worked
/// inside it takes it down with it, unless the site is not its side's, which
/// then stands halted for its own; one that was to become the site takes it
/// down with it, whatever side the site is.
fn leave_site(world: &mut World, entity: Entity, site: SimulationId) {
    let departure = leave_crew(world, site, entity);
    work::leave(world, entity, Beside::Container(site));
    match departure {
        Departure::LastOut => {}
        Departure::OthersRemain | Departure::JobGone => return,
    }
    match attendance(world, entity) {
        BuilderAttendance::Crew(WorkPresence::Present { .. } | WorkPresence::Attached(_)) => {
            halt_site(world, site)
        }
        BuilderAttendance::Crew(WorkPresence::Hidden { .. }) => {
            let building = world
                .resource::<EntityIndex>()
                .alive(site)
                .expect("the last builder leaves a site that still stands");
            if entity_def::same_owner(world, entity, building) {
                abandon_site(world, site)
            } else {
                halt_site(world, site)
            }
        }
        BuilderAttendance::Consumed => abandon_site(world, site),
        BuilderAttendance::Unattended => {
            unreachable!("an unattended builder is never on a crew")
        }
    }
}

/// Ends a crew builder's order on `site`, which has completed: one that attends
/// steps back out beside it, or stays where it stands;
/// one consumed by its work is despawned as consumed, and its order finishes
/// dying. Every early end of the work — a cancel, a destroyed site — brings a
/// builder back instead, through [`work::leave`].
fn leave_finished_site(world: &mut World, entity: Entity, site: SimulationId) -> Processing {
    match attendance(world, entity) {
        BuilderAttendance::Crew(_) => {
            work::leave(world, entity, Beside::Container(site));
            Processing::state(OrderState::Finished)
        }
        BuilderAttendance::Consumed => {
            spawn::despawn_entity(world, entity, DeathCause::Consumed);
            Processing::finished_dying()
        }
        BuilderAttendance::Unattended => {
            unreachable!("an unattended builder's order ends at placement")
        }
    }
}

/// Refuses a builder whose attendance would take it off the map from under
/// others seated on it; one that leaves the site to itself is never refused.
fn requires_free_to_attend(world: &World, entity: Entity) -> Result<(), Refusal> {
    match attendance(world, entity).presence() {
        Some(presence) => work::requires_free_to_hide(world, entity, &presence),
        None => Ok(()),
    }
}

/// How the builder relates to the site its order is on.
fn attendance(world: &World, entity: Entity) -> BuilderAttendance {
    entity_def::builder_attendance(world, entity)
        .expect("a build order only starts on an entity that can build")
        .clone()
}
