//! Build order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeSet;

use bevy_ecs::{
    entity::Entity,
    query::{With, Without},
    world::World,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};

use super::{
    chase::{self, Destination},
    crew::{self, Departure},
    orders::{self, Processing, Refusal},
    work,
};
use crate::{
    components::{
        build::{BuildComponent, SiteWork, UnderConstructionComponent},
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause, SpendCause},
    fields,
    map::Map,
    order::Order,
    requirements,
    resources::{self, PlayerResources},
    session::player_id::PlayerId,
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
    supply,
};
use ferrets_content::{
    build::BuilderAttendance, entity_stats::EntityStatId, registry::ContentRegistry,
    work::WorkPresence,
};

/// Whether `entity` may start this Build: its type raises the ordered type,
/// that type is constructible, and it operates. Whether the ground admits the
/// site is decided on arrival.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let (type_name, _) = order.build_params().expect("Build order must have params");
    if !entity_def::of(world, entity)
        .builder
        .as_ref()
        .is_some_and(|builder_def| builder_def.can_build(type_name))
    {
        return Err(Refusal::Incapable);
    }
    let constructible = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .is_some_and(|def| def.build_time.is_some());
    if !constructible {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)
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
/// Construction stops immediately under both policies. The site itself is only torn
/// down and refunded by the last builder to leave it, so pulling one worker off a
/// shared site leaves the rest to finish it.
pub fn cancel_processing(
    entity: Entity,
    order: &Order,
    _policy: CancelPolicy,
    entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    // A queued entry was never prepared: the driver on the entity, if any,
    // belongs to the build under way in front of it.
    match entry_state {
        OrderState::New => return OrderState::Finished,
        OrderState::InProcessing | OrderState::Suspended => {}
        OrderState::Finished => unreachable!("Finished entries never stay in the queue"),
    }
    let Some(build_component) = world.entity_mut(entity).take::<BuildComponent>() else {
        return OrderState::Finished;
    };

    if let Some(building_id) = build_component.building {
        let (type_name, position) = order.build_params().expect("Build order must have params");
        let size = world
            .resource::<ContentRegistry>()
            .entity(type_name)
            .expect("type checked in prepare")
            .location
            .expect("validated content defines a location")
            .size();

        match leave_crew(world, building_id, entity) {
            Departure::LastOut => abandon_site(world, entity, building_id, type_name),
            Departure::OthersRemain | Departure::JobGone => {}
        }
        work::leave(world, entity, CellPos::from(position), size);
    }

    OrderState::Finished
}

/// Advance a Build order by one tick.
///
/// Until the site is taken up: walk to within the builder's `build_range` of it
/// (suspending on a chase move), then either join a matching site already under way
/// there, or pay the cost and place one. The order finishes early if the site is
/// blocked — which includes a builder of its own standing in the footprint, since a
/// builder that works in the open is never moved out of the way — if the site is
/// already held by a builder that will not share it, or if the cost cannot be paid.
///
/// After that: every builder on the site advances the same progress counter by one
/// tick's work. When the build time is reached the construction marker is removed,
/// and a builder that raised the site from inside comes back out beside it.
///
/// A builder that leaves the site unattended is done the moment the site
/// stands: the site advances itself from there (see
/// [`advance_sites_without_builder`]). One consumed by its work is despawned
/// as the site completes instead of stepping back out.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let (type_name, position) = order.build_params().expect("Build order must have params");

    let Some(mut build_component) = world.entity_mut(entity).take::<BuildComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let (build_time, building_location_def, cost) = {
        let registry = world.resource::<ContentRegistry>();
        let type_def = registry.entity(type_name).expect("type checked in prepare");
        (
            type_def.build_time.expect("type checked in prepare"),
            type_def
                .location
                .expect("validated content defines a location"),
            type_def.cost.clone(),
        )
    };
    let site_origin = CellPos::from(position);
    let size = building_location_def.size();

    let Some(building_id) = build_component.building else {
        // Walk-and-place phase.
        let projection = world.resource::<Map>().projection();

        let (chaser_position, chaser_size) = entity_def::footprint(world, entity);
        match chase::advance(
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
        if let Some(site) = site_under_way_at(world, type_name, site_origin, owner) {
            if site_excludes(world, site, entity) {
                return Processing::state(OrderState::Finished);
            }
            // Nothing is placed and nothing is paid for — joining is just taking up
            // a position on somebody else's job.
            join_crew(world, site, entity);
            enter_site(world, entity);
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
            if !requirements::met(world, player, &def.requires) {
                return Processing::state(OrderState::Finished);
            }
            if !world
                .resource::<PlayerResources>()
                .can_afford(player, &cost)
            {
                return Processing::state(OrderState::Finished);
            }
        }
        // Fields gate the placement the same way: judged at the raise, never
        // again for a site already standing.
        {
            let def = world
                .resource::<ContentRegistry>()
                .entity(type_name)
                .expect("type checked in prepare");
            if !fields::allows_placement(world, owner, def, CellPos::from(position)) {
                return Processing::state(OrderState::Finished);
            }
        }

        // A builder that disappears into its work leaves the map now, which frees any
        // of the site's cells it was standing on. One that stays in the open blocks
        // the site the way anything else standing there would.
        enter_site(world, entity);

        let builder = entity_def::simulation_id(world, entity);
        let work = match attendance(world, entity) {
            BuilderAttendance::Crew(_) | BuilderAttendance::Consumed => SiteWork::Crew {
                builders: BTreeSet::from([builder]),
            },
            BuilderAttendance::Unattended => SiteWork::Unattended { founder: builder },
        };
        let placed = spawn::spawn_entity(
            world,
            type_name,
            position,
            owner,
            SpawnCause::Founded { builder },
            FieldReach::Initial,
        );
        let Some((building, building_sim_id)) = placed else {
            // Site blocked — give up, and bring back a builder that had already
            // stepped inside.
            work::leave(world, entity, site_origin, size);
            return Processing::state(OrderState::Finished);
        };

        world
            .entity_mut(building)
            .insert(UnderConstructionComponent {
                progress: 0,
                work: work.clone(),
            });
        if let Some(player) = owner {
            resources::charge(
                world,
                player,
                cost,
                SpendCause::Construction {
                    site: building_sim_id,
                },
            );
        }

        match work {
            SiteWork::Crew { .. } => {}
            SiteWork::Unattended { .. } => {
                return Processing::state(OrderState::Finished);
            }
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
        work::leave(world, entity, site_origin, size);
        return Processing::state(OrderState::Finished);
    };

    // Another builder on the same site may have finished it first.
    let mut building_mut = world.entity_mut(building);
    let Some(mut progress) = building_mut.get_mut::<UnderConstructionComponent>() else {
        return leave_finished_site(world, entity, site_origin, size);
    };
    progress.progress += 1;

    if progress.progress >= build_time {
        complete_site(world, building, entity_def::simulation_id(world, entity));
        return leave_finished_site(world, entity, site_origin, size);
    }

    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// Advances every unattended site by one tick, completing the ones that reach
/// their build time. A site a crew works is left to its crew.
///
/// Sites are visited in ascending simulation-id order.
pub fn advance_sites_without_builder(world: &mut World) {
    let mut unattended: Vec<(SimulationId, Entity, SimulationId)> = world
        .query_filtered::<(Entity, &EntityInfoComponent, &UnderConstructionComponent), Without<DyingComponent>>()
        .iter(world)
        .filter_map(|(building, info, site)| match site.work {
            SiteWork::Crew { .. } => None,
            SiteWork::Unattended { founder } => Some((info.id(), building, founder)),
        })
        .collect();
    unattended.sort_unstable_by_key(|&(id, _, _)| id);
    for (_, building, founder) in unattended {
        let build_time = entity_def::of(world, building)
            .build_time
            .expect("a site's type is constructible");

        let mut building_mut = world.entity_mut(building);
        let mut site = building_mut
            .get_mut::<UnderConstructionComponent>()
            .expect("checked above");
        site.progress += 1;
        if site.progress >= build_time {
            complete_site(world, building, founder);
        }
    }
}

/// Removes the construction marker from `building` and announces the
/// completion, naming `builder` — whoever worked the completing tick, or the
/// founder of a site that raised itself.
fn complete_site(world: &mut World, building: Entity, builder: SimulationId) {
    world
        .entity_mut(building)
        .remove::<UnderConstructionComponent>();
    let announced = SimulationEvent::ConstructionCompleted {
        building: entity_def::simulation_id(world, building),
        builder,
    };
    world.resource_mut::<EventRecord>().emit(announced);
}

/// Destroys an unfinished site and refunds what it cost, called by the last builder
/// to walk away from it.
fn abandon_site(world: &mut World, entity: Entity, site: SimulationId, type_name: &str) {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("the last builder leaves a site that still stands");
    assert!(
        world
            .entity(building)
            .contains::<UnderConstructionComponent>(),
        "the last builder leaves a site still under construction"
    );

    let cost = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("type checked in prepare")
        .cost
        .clone();

    spawn::despawn_entity(world, building, DeathCause::Cancelled);
    if let Some(player) = entity_def::owner(world, entity) {
        resources::refund(world, player, cost, SpendCause::Construction { site });
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
    ), (With<UnderConstructionComponent>, Without<DyingComponent>)>();

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

/// Whether `entity` is shut out of `site`: by the crew already on it, or
/// because the site takes no crew.
fn site_excludes(world: &World, site: SimulationId, entity: Entity) -> bool {
    match world.resource::<EntityIndex>().alive(site) {
        Some(building) => {
            crew::excludes::<UnderConstructionComponent>(world, building, entity, shares_sites)
        }
        None => false,
    }
}

/// Joins the crew on `site`.
///
/// The site's marker is the construction itself, raised with the first builder — so a
/// newcomer joins what is there and never marks anything.
fn join_crew(world: &mut World, site: SimulationId, entity: Entity) {
    let building = world
        .resource::<EntityIndex>()
        .alive(site)
        .expect("a crew is joined on a site found standing this tick");
    crew::join_existing::<UnderConstructionComponent>(world, building, entity);
}

/// Drops out of the crew on `site` — the last builder off an unfinished site is
/// the one that tears it down.
///
/// The marker stays behind either way: it carries the work raised so far, which is not
/// the crew's to take away. A site that finished or was destroyed in the meantime is
/// gone as a job and is nobody's to tear down.
fn leave_crew(world: &mut World, site: SimulationId, entity: Entity) -> Departure {
    match world.resource::<EntityIndex>().alive(site) {
        Some(building) => crew::leave::<UnderConstructionComponent>(world, building, entity),
        None => Departure::JobGone,
    }
}

/// Whether an entity's build capability lets several builders share one site.
fn shares_sites(world: &World, entity: Entity) -> bool {
    match attendance(world, entity) {
        BuilderAttendance::Crew(presence) => presence.stacks(),
        BuilderAttendance::Unattended | BuilderAttendance::Consumed => false,
    }
}

/// Puts the builder where its attendance has it stand as it takes up a site:
/// a crew builder as its presence says, one consumed by the site hidden inside
/// it, which frees any of the site's cells it was standing on. One that leaves
/// the site unattended stays exactly where it walked to.
fn enter_site(world: &mut World, entity: Entity) {
    let presence = match attendance(world, entity) {
        BuilderAttendance::Crew(presence) => presence,
        BuilderAttendance::Consumed => WorkPresence::Hidden,
        BuilderAttendance::Unattended => return,
    };
    work::enter(world, entity, presence);
}

/// Ends a crew builder's order on a site that has completed: one that attends
/// steps back out beside the footprint at `around`, or stays where it stands;
/// one consumed by its work is despawned as consumed, and its order finishes
/// dying. Every early end of the work — a cancel, a destroyed site — brings a
/// builder back instead, through [`work::leave`].
fn leave_finished_site(
    world: &mut World,
    entity: Entity,
    around: CellPos,
    around_size: CellSize,
) -> Processing {
    match attendance(world, entity) {
        BuilderAttendance::Crew(_) => {
            work::leave(world, entity, around, around_size);
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

/// How the builder relates to the site its order is on.
fn attendance(world: &World, entity: Entity) -> BuilderAttendance {
    entity_def::builder_attendance(world, entity)
        .expect("a build order only starts on an entity that can build")
}
