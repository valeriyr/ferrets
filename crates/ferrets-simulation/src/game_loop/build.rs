//! Build order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeSet;

use bevy_ecs::{
    entity::Entity,
    query::{With, Without},
    world::World,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};

use super::{
    chase::{self, Destination},
    crew::{self, Departure},
    orders::{self, Processing, Refusal},
    work,
};
use crate::{
    annex, berths,
    beside::Beside,
    components::{
        build::BuildComponent,
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        hidden::{self, Inside},
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        resource::ResourceSourceComponent,
        rise::{self, RiseAttendance, RisingComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpendCause},
    fields,
    map::Map,
    order::Order,
    requirements,
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

/// Whether `entity` may start this Build: its type raises the ordered type,
/// that type rises, it operates, and attending the site would not take it off
/// the map from under others seated on it. Whether the ground admits the site
/// is decided on arrival.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let (type_name, _) = order.build_params().expect("Build order must have params");
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
    requires_free_to_attend(world, entity)
}

/// Called once when a Build order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    world.entity_mut(entity).insert(BuildComponent::default());
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

    if let Some(building_id) = build_component.building {
        let departure = leave_crew(world, building_id, entity);
        // The builder steps out beside the site as it stands, before the last
        // one out decides its fate.
        work::leave(world, entity, Beside::Container(building_id));
        // The last builder off a site decides its fate by how it attended
        // it: one that worked inside the site, or was to become it, takes the
        // site down with it; one that stood on or beside it leaves the site
        // standing for whoever takes it up next.
        match departure {
            Departure::LastOut => match attendance(world, entity) {
                BuilderAttendance::Crew(WorkPresence::Hidden { .. })
                | BuilderAttendance::Consumed => abandon_site(world, entity, building_id),
                BuilderAttendance::Crew(
                    WorkPresence::Present { .. } | WorkPresence::Attached(_),
                ) => halt_site(world, building_id),
                BuilderAttendance::Unattended => {
                    unreachable!("an unattended builder is never on a crew")
                }
            },
            Departure::OthersRemain | Departure::JobGone => {}
        }
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
/// (suspending on a chase move), then either join a matching site already under way
/// there, or pay the price and place one. The order finishes early if the site is
/// blocked — which includes a builder of its own standing in the footprint, since a
/// builder that works in the open is never moved out of the way — if the site is
/// already held by a builder that will not share it, if the price cannot be paid,
/// or if others sat down on the builder since the order started and its
/// attendance could take it off the map from under them.
///
/// After that: every builder on the site advances the same progress counter by one
/// tick's work. When the rise time is reached the rising marker is removed,
/// and a builder that raised the site from inside comes back out beside it.
///
/// A builder that leaves the site unattended is done the moment the site
/// stands: the site advances itself from there (see
/// [`advance_sites_without_builder`]). One consumed by its work is despawned
/// as the site completes instead of stepping back out. A site whose crew left
/// it halted is taken up again by the next builder sent to it.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let (type_name, position) = order.build_params().expect("Build order must have params");

    let Some(mut build_component) = world.entity_mut(entity).take::<BuildComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let (rise_time, building_location_def, price) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name).expect("type checked in prepare");
        (
            type_def.rise_time().expect("type checked in prepare"),
            type_def
                .location
                .expect("validated content defines a location"),
            type_def.price.clone(),
        )
    };
    let site_anchor = CellPos::from(position);
    let size = building_location_def.size();

    let Some(building_id) = build_component.building else {
        // Walk-and-place phase.
        let projection = world.resource::<Map>().projection();

        let (chaser_position, chaser_size) = entity_def::footprint(world, entity);
        match chase::advance_to_point(
            &mut build_component.last_chase,
            projection,
            chaser_position,
            chaser_size,
            position,
            size,
            entity_def::effective_stat_u32(world, entity, EntityStatId::BUILD_RANGE),
        ) {
            Destination::OutOfReach => return Processing::state(OrderState::Finished),
            Destination::Walk(move_order) => {
                world.entity_mut(entity).insert(build_component);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }

        chase::face(world, entity, position, size);

        let owner = entity_def::owner(world, entity);

        // Work already under way here is joined rather than started again: the site
        // holds the cells, so a second placement could only ever fail.
        if let Some(site) = site_under_way_at(world, type_name, site_anchor, owner) {
            if site_excludes(world, site, entity) {
                return Processing::state(OrderState::Finished);
            }
            // A builder others sat down on since the order started gives
            // the order up.
            match requires_free_to_attend(world, entity) {
                Ok(()) => {}
                Err(_) => return Processing::state(OrderState::Finished),
            }
            // Nothing is placed and nothing is paid for — joining is just taking up
            // a position on somebody else's job.
            let building = world
                .resource::<EntityIndex>()
                .alive(site)
                .expect("a site is taken up only when found standing this tick");
            let taken = join_site(world, site, entity);
            enter_site(world, entity, building);
            // A builder that leaves the site to itself is done the moment it
            // has taken it up, exactly as it is done the moment it raises one.
            match taken {
                Taken::OnCrew => {}
                Taken::LeftToItself => return Processing::state(OrderState::Finished),
            }
            build_component.building = Some(site);
            world.entity_mut(entity).insert(build_component);
            return Processing::state(OrderState::InProcessing);
        }

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
        let overbuilt = match overbuilds {
            Some(over) => {
                let Some(source) = source_under(world, &over, CellRect::new(site_anchor, size))
                else {
                    return Processing::state(OrderState::Finished);
                };
                Some(source)
            }
            None => None,
        };

        // A builder others sat down on since the order started gives the
        // order up before raising anything.
        match requires_free_to_attend(world, entity) {
            Ok(()) => {}
            Err(_) => return Processing::state(OrderState::Finished),
        }

        // A builder that disappears into its work leaves the grid now, which
        // frees any of the site's cells it was standing on. One that stays in
        // the open blocks the site the way anything else standing there would;
        // one that attaches to the site sits in its berths once it stands. The
        // site is not raised yet, so the builder is inside nothing until it
        // takes the site up.
        match attendance(world, entity) {
            BuilderAttendance::Crew(WorkPresence::Hidden { .. }) | BuilderAttendance::Consumed => {
                spawn::hide_entity(world, entity, Inside::Nothing);
            }
            BuilderAttendance::Crew(WorkPresence::Present { .. } | WorkPresence::Attached(_))
            | BuilderAttendance::Unattended => {}
        }

        let builder = entity_def::simulation_id(world, entity);
        let rise_attendance = founding_attendance(world, entity, builder);
        // The source's footprint comes off the grid so the site can take its
        // cells; a site refused anyway puts it straight back.
        if let Some(source) = overbuilt {
            spawn::lift_footprint(world, source);
        }
        let placed = spawn::spawn_founded(
            world,
            type_name,
            position,
            owner,
            FieldReach::Initial,
            builder,
            rise_attendance.clone(),
        );
        let Some((building, building_sim_id)) = placed else {
            // Site blocked — give up, and bring back a builder that had already
            // stepped inside.
            if let Some(source) = overbuilt {
                spawn::restore_footprint(world, source);
            }
            work::leave(world, entity, Beside::WentIn);
            return Processing::state(OrderState::Finished);
        };
        if let Some(source) = overbuilt {
            spawn::cover_source(world, source, building);
        }

        // A builder hidden before the site was raised is inside it now; one in
        // the open or in a berth takes it up as it would one under way; one
        // that leaves it to itself stays where it walked to.
        match attendance(world, entity) {
            BuilderAttendance::Crew(WorkPresence::Hidden { .. }) | BuilderAttendance::Consumed => {
                hidden::record_inside(world, entity, Inside::Job(building_sim_id));
            }
            BuilderAttendance::Crew(WorkPresence::Present { .. } | WorkPresence::Attached(_)) => {
                enter_site(world, entity, building);
            }
            BuilderAttendance::Unattended => {}
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
        build_component.building = Some(building_sim_id);
        world.entity_mut(entity).insert(build_component);
        return Processing::state(OrderState::InProcessing);
    };

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
    let builder = entity_def::simulation_id(world, entity);
    match put_in_work(world, building, rise_time, builder) {
        Worked::Risen => return leave_finished_site(world, entity, building_id),
        Worked::Going => {}
    }

    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// Advances every unattended rising entity by one tick, marking risen the
/// ones that reach their rise time. One a crew works is left to its crew,
/// and a halted one waits.
///
/// Rising entities are visited in ascending simulation-id order.
pub fn advance_sites_without_builder(world: &mut World) {
    let mut unattended: Vec<(SimulationId, Entity, SimulationId)> = world
        .query_filtered::<(Entity, &EntityInfoComponent, &RisingComponent), Without<DyingComponent>>()
        .iter(world)
        .filter_map(|(entity, info, rising)| match rising.attendance {
            RiseAttendance::Crew { .. } | RiseAttendance::Halted => None,
            RiseAttendance::Unattended { founder } => Some((info.id(), entity, founder)),
        })
        .collect();
    unattended.sort_unstable_by_key(|&(id, _, _)| id);
    for (_, entity, founder) in unattended {
        let rise_time = entity_def::of(world, entity)
            .rise_time()
            .expect("an unattended rising entity is of a type that rises");

        match put_in_work(world, entity, rise_time, founder) {
            Worked::Going | Worked::Risen => {}
        }
    }
}

/// Lines up each pool of every rising entity that follows its work under the
/// maxima just folded — see [`rise::rise`] — and settles the pools still
/// following work on each entity risen since the last fold — see
/// [`rise::settle_risen`].
pub fn rise_sites(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !entity_def::follows_work(world, entity) {
            continue;
        }
        if entity_def::rising(world, entity) {
            rise::rise(world, entity);
        } else {
            rise::settle_risen(world, entity);
        }
    }
}

/// Removes the unfinished `building` from the map, paying nothing back.
pub(super) fn tear_down_site(world: &mut World, building: Entity) {
    spawn::despawn_entity(world, building, DeathCause::Canceled);
}

/// Marks `entity` as risen — see [`rise::mark_as_risen`] — and announces it,
/// naming `by`: whoever worked the last tick, or the founder of an entity
/// that rose unattended.
fn complete_site(world: &mut World, entity: Entity, by: SimulationId) {
    rise::mark_as_risen(world, entity);
    let announced = SimulationEvent::EntityRisen {
        entity: entity_def::simulation_id(world, entity),
        by,
    };
    world.resource_mut::<EventRecord>().emit(announced);
}

/// Destroys an unfinished site, called by the last builder to walk away from a
/// site its attendance does not leave standing. What the site cost stays
/// spent: only a cancel aimed at the site pays it back.
fn abandon_site(world: &mut World, entity: Entity, site: SimulationId) {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("the last builder leaves a site that still stands");
    assert!(
        entity_def::rising(world, building),
        "the last builder leaves a site still rising"
    );
    debug_assert_eq!(
        entity_def::owner(world, building),
        entity_def::owner(world, entity),
        "a builder works only its owner's sites"
    );
    tear_down_site(world, building);
}

/// Leaves the unfinished `site` standing without a crew, its progress kept,
/// called by the last builder to walk away from a site its attendance leaves
/// standing.
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
            let taken = founding_attendance(world, entity, builder);
            let on_crew = matches!(taken, RiseAttendance::Crew { .. });
            world
                .entity_mut(building)
                .get_mut::<RisingComponent>()
                .expect("checked above")
                .attendance = taken;
            match on_crew {
                true => Taken::OnCrew,
                false => Taken::LeftToItself,
            }
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

/// Drops out of the crew on `site` — the last builder off an unfinished site is
/// the one that tears it down.
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
/// it. One that leaves the site unattended stays exactly where it walked to.
fn enter_site(world: &mut World, entity: Entity, site: Entity) {
    if let Some(presence) = attendance(world, entity).presence() {
        work::enter(world, entity, &presence, site);
    }
}

/// The resource source of type `over` standing exactly on `footprint`, if one
/// does.
fn source_under(world: &mut World, over: &str, footprint: CellRect) -> Option<Entity> {
    let mut query = world.query_filtered::<(Entity, &EntityInfoComponent), (
        With<ResourceSourceComponent>,
        With<LocationComponent>,
        Without<DyingComponent>,
    )>();
    // The lowest id wins, so a shared outcome never rests on query order.
    query
        .iter(world)
        .filter(|(entity, info)| {
            info.type_name() == over
                && entity_def::stands_on_grid(world, *entity)
                && entity_def::footprint_rect(world, *entity) == footprint
        })
        .min_by_key(|(_, info)| info.id())
        .map(|(entity, _)| entity)
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

/// What a tick of work did to a rising entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Worked {
    /// The entity goes on rising.
    Going,
    /// The work reached the rise time and the entity has risen.
    Risen,
}

/// Puts one tick of work into the rising `entity`: each pool following its
/// work moves along its line, and once the work reaches `rise_time` the
/// entity has risen, named for `by`.
fn put_in_work(world: &mut World, entity: Entity, rise_time: u32, by: SimulationId) -> Worked {
    let progress = {
        let mut rising = world
            .get_mut::<RisingComponent>(entity)
            .expect("work is put into an entity still rising");
        rising.progress += 1;
        rising.progress
    };
    if entity_def::follows_work(world, entity) {
        rise::rise(world, entity);
    }
    if progress >= rise_time {
        complete_site(world, entity, by);
        Worked::Risen
    } else {
        Worked::Going
    }
}
