//! Build order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_pathfinder::nav_pos::NavPos;

use super::chase::{self, Destination};
use super::orders::Processing;
use crate::{
    components::{
        build::{BuildComponent, UnderConstructionComponent},
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
    },
    content::registry::ContentRegistry,
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    resources::PlayerResources,
    spawn,
};

/// How close the builder must be to the construction site, in grid cells.
const BUILD_DISTANCE: u32 = 1;

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
/// Construction stops immediately under both policies. If the building was already
/// placed, it is destroyed, its cost is refunded, and the builder reappears next
/// to the site.
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
        let (cost, size) = {
            let registry = world.resource::<ContentRegistry>();
            let def = registry.entity(type_name).expect("type checked in prepare");
            (
                def.cost.clone(),
                def.location
                    .expect("validated content defines a location")
                    .size(),
            )
        };

        if let Some(building) = world.resource::<EntityIndex>().alive(building_id) {
            spawn::destroy_entity(world, building);
        }
        if let Some(player) = world
            .entity(entity)
            .get::<OwnerComponent>()
            .map(|o| o.player())
        {
            world
                .resource_mut::<PlayerResources>()
                .refund(player, &cost);
        }
        // Cancellation cannot retry here, so the reveal is queued for later ticks
        // if no cell is free now.
        spawn::reveal_entity_near_or_retry(world, entity, NavPos::from(position), size);
    }

    OrderState::Finished
}

/// Advance a Build order by one tick.
///
/// Until the building is placed: walk to within `BUILD_DISTANCE` of the site
/// (suspending on a chase move), then pay the cost, hide the builder inside the
/// site, and spawn the building under construction. The order finishes early if
/// the site is blocked or the cost cannot be paid.
///
/// After placement: construction progresses each tick. When the build time is
/// reached, the construction marker is removed and the builder reappears on the
/// nearest free cell around the building.
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
    let site_origin = NavPos::from(position);

    let Some(building_id) = build_component.building else {
        // Walk-and-place phase.
        let builder_position = world
            .entity(entity)
            .get::<LocationComponent>()
            .unwrap()
            .position;
        let projection = world.resource::<Map>().projection();

        match chase::advance(
            &mut build_component.last_chase,
            projection,
            builder_position,
            position,
            building_location_def.size(),
            BUILD_DISTANCE,
        ) {
            Destination::OutOfReach => return Processing::state(OrderState::Finished),
            Destination::Walk(move_order) => {
                world.entity_mut(entity).insert(build_component);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }

        chase::face(world, entity, position);

        let owner = world
            .entity(entity)
            .get::<OwnerComponent>()
            .map(|o| o.player());
        if let Some(player) = owner
            && !world
                .resource::<PlayerResources>()
                .can_afford(player, &cost)
        {
            return Processing::state(OrderState::Finished);
        }

        // Hide the builder before checking the site so its own cell does not
        // block the footprint.
        spawn::hide_entity(world, entity);

        let placed = spawn::spawn_entity(world, type_name, position, owner);
        let Some((building, building_sim_id)) = placed else {
            // Site blocked — bring the builder back and give up. This path
            // cannot retry, so the reveal is queued if no cell is free now.
            spawn::reveal_entity_near_or_retry(
                world,
                entity,
                site_origin,
                building_location_def.size(),
            );
            return Processing::state(OrderState::Finished);
        };

        world
            .entity_mut(building)
            .insert(UnderConstructionComponent);
        if let Some(player) = owner {
            world
                .resource_mut::<PlayerResources>()
                .subtract(player, &cost);
        }

        build_component.building = Some(building_sim_id);
        world.entity_mut(entity).insert(build_component);
        return Processing::state(OrderState::InProcessing);
    };

    // Construction phase.
    let Some(building) = world.resource::<EntityIndex>().alive(building_id) else {
        // The building was destroyed mid-construction. This path cannot retry,
        // so the reveal is queued if no cell is free now.
        spawn::reveal_entity_near_or_retry(
            world,
            entity,
            site_origin,
            building_location_def.size(),
        );
        return Processing::state(OrderState::Finished);
    };

    if build_component.progress < build_time {
        build_component.progress += 1;
    }

    if build_component.progress >= build_time {
        // No free cell to reappear on — stay inside and retry every tick.
        if !spawn::reveal_entity_near(world, entity, site_origin, building_location_def.size()) {
            world.entity_mut(entity).insert(build_component);
            return Processing::state(OrderState::InProcessing);
        }
        world
            .entity_mut(building)
            .remove::<UnderConstructionComponent>();
        return Processing::state(OrderState::Finished);
    }

    world.entity_mut(entity).insert(build_component);
    Processing::state(OrderState::InProcessing)
}
