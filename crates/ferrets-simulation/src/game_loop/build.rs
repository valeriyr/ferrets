//! Build order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use std::collections::BTreeSet;

use bevy_ecs::{
    entity::Entity,
    query::{With, Without},
    world::World,
};
use ferrets_geometry::cell_pos::CellPos;

use super::{
    chase::{self, Destination},
    crew,
    orders::Processing,
    work,
};
use crate::{
    components::{
        build::{BuildComponent, UnderConstructionComponent},
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
    },
    content::{entity_stats::EntityStatId, registry::ContentRegistry, work::WorkPresence},
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    requirements,
    resources::PlayerResources,
    session::player_slot::PlayerId,
    simulation_id::SimulationId,
    spawn, supply,
};

/// Called once when a Build order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot build or the ordered type is not constructible.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let (type_name, _) = order.build_params().expect("Build order must have params");

    if !entity_def::of(world, entity)
        .builder
        .as_ref()
        .is_some_and(|builder_def| builder_def.can_build(type_name))
    {
        return OrderState::Finished;
    }
    let constructible = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .is_some_and(|def| def.build_time.is_some());
    if !constructible {
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
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
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

        if leave_crew(world, building_id, entity) {
            abandon_site(world, entity, building_id, type_name);
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

        match chase::advance(
            &mut build_component.last_chase,
            projection,
            entity_def::position(world, entity),
            position,
            size,
            work::reach(world, entity, EntityStatId::BUILD_RANGE),
        ) {
            Destination::OutOfReach => return Processing::state(OrderState::Finished),
            Destination::Walk(move_order) => {
                world.entity_mut(entity).insert(build_component);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }

        chase::face(world, entity, position, size);

        let owner = world
            .entity(entity)
            .get::<OwnerComponent>()
            .map(|o| o.player());

        // Work already under way here is joined rather than started again: the site
        // holds the cells, so a second placement could only ever fail.
        if let Some(site) = site_under_way_at(world, type_name, site_origin, owner) {
            if site_excludes(world, site, entity) {
                return Processing::state(OrderState::Finished);
            }
            // Nothing is placed and nothing is paid for — joining is just taking up
            // a position on somebody else's job.
            join_crew(world, site, entity);
            work::enter(world, entity, presence(world, entity));
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

        // A builder that disappears into its work leaves the map now, which frees any
        // of the site's cells it was standing on. One that stays in the open blocks
        // the site the way anything else standing there would.
        work::enter(world, entity, presence(world, entity));

        let placed = spawn::spawn_entity(world, type_name, position, owner);
        let Some((building, building_sim_id)) = placed else {
            // Site blocked — give up, and bring back a builder that had already
            // stepped inside.
            work::leave(world, entity, site_origin, size);
            return Processing::state(OrderState::Finished);
        };

        let builder_id = entity_def::simulation_id(world, entity);
        world
            .entity_mut(building)
            .insert(UnderConstructionComponent {
                progress: 0,
                builders: BTreeSet::from([builder_id]),
            });
        if let Some(player) = owner {
            world
                .resource_mut::<PlayerResources>()
                .subtract(player, &cost);
        }

        build_component.building = Some(building_sim_id);
        world.entity_mut(entity).insert(build_component);
        return Processing::state(OrderState::InProcessing);
    };

    // Construction phase: one tick of this builder's work, whoever else is on the
    // site. Every way out of it leaves the site, which is a no-op for a builder that
    // never stepped inside.
    let Some(building) = world.resource::<EntityIndex>().alive(building_id) else {
        // The building was destroyed mid-construction.
        work::leave(world, entity, site_origin, size);
        return Processing::state(OrderState::Finished);
    };

    // Another builder on the same site may have finished it first.
    let mut building_mut = world.entity_mut(building);
    let Some(mut progress) = building_mut.get_mut::<UnderConstructionComponent>() else {
        work::leave(world, entity, site_origin, size);
        return Processing::state(OrderState::Finished);
    };
    progress.progress += 1;

    if progress.progress >= build_time {
        world
            .entity_mut(building)
            .remove::<UnderConstructionComponent>();
        work::leave(world, entity, site_origin, size);
        return Processing::state(OrderState::Finished);
    }

    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}

/// Destroys an unfinished site and refunds what it cost, called by the last builder
/// to walk away from it.
///
/// A site that finished in the meantime is left alone: it is a building now, not an
/// abandoned job.
fn abandon_site(world: &mut World, entity: Entity, site: SimulationId, type_name: &str) {
    let Some(building) = world.resource::<EntityIndex>().alive(site) else {
        return;
    };
    if !world
        .entity(building)
        .contains::<UnderConstructionComponent>()
    {
        return;
    }

    let cost = world
        .resource::<ContentRegistry>()
        .entity(type_name)
        .expect("type checked in prepare")
        .cost
        .clone();

    spawn::destroy_entity(world, building);
    if let Some(player) = world
        .entity(entity)
        .get::<OwnerComponent>()
        .map(|o| o.player())
    {
        world
            .resource_mut::<PlayerResources>()
            .refund(player, &cost);
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

/// Whether `entity` is shut out of `site` by the crew already on it.
fn site_excludes(world: &World, site: SimulationId, entity: Entity) -> bool {
    let Some(building) = world.resource::<EntityIndex>().alive(site) else {
        return false;
    };
    crew::excludes::<UnderConstructionComponent>(world, building, entity, shares_sites)
}

/// Joins the crew on `site`.
///
/// The site's marker is the construction itself, raised with the first builder — so a
/// newcomer joins what is there and never marks anything.
fn join_crew(world: &mut World, site: SimulationId, entity: Entity) {
    let Some(building) = world.resource::<EntityIndex>().alive(site) else {
        return;
    };
    crew::join_existing::<UnderConstructionComponent>(world, building, entity);
}

/// Drops out of the crew on `site`, and reports whether that leaves it unmanned — the
/// last builder off an unfinished site is the one that tears it down.
///
/// The marker stays behind either way: it carries the work raised so far, which is not
/// the crew's to take away. A site that finished or was destroyed in the meantime has
/// no crew to leave and is nobody's to tear down.
fn leave_crew(world: &mut World, site: SimulationId, entity: Entity) -> bool {
    match world.resource::<EntityIndex>().alive(site) {
        Some(building) => crew::leave::<UnderConstructionComponent>(world, building, entity),
        None => false,
    }
}

/// Whether an entity's build capability lets several builders share one site.
fn shares_sites(world: &World, entity: Entity) -> bool {
    presence(world, entity).stacks()
}

/// Where the builder stands while its site goes up.
fn presence(world: &World, entity: Entity) -> WorkPresence {
    entity_def::of(world, entity)
        .builder
        .as_ref()
        .expect("a build order only starts on an entity that can build")
        .presence()
}
