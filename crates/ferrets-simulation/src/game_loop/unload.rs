//! Unload order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::{
    chase::{self, Destination},
    orders::Processing,
    rally,
};
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        transport::{
            BoardedComponent, GarrisonFireComponent, TransporterComponent, UnloadComponent,
        },
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    simulation_id::SimulationId,
    spawn,
};
use ferrets_content::entity_stats::EntityStatId;

/// Called once when an Unload order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity is not a transporter or holds nobody.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let at = order
        .unload_at()
        .expect("Unload order carries a destination");

    if !entity_def::of(world, entity).can_transport() {
        return OrderState::Finished;
    }
    if world
        .entity(entity)
        .get::<TransporterComponent>()
        .is_none_or(|transporter| transporter.passengers.is_empty())
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(UnloadComponent::new(at));
    OrderState::InProcessing
}

/// Called when an Unload order resumes from `Suspended` (its walk toward the
/// destination just finished). The driver component survives suspension.
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Unload entry that has a cancel policy.
///
/// Unloading stops immediately under both policies: whoever is out is out, and
/// whoever is still aboard stays aboard.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<UnloadComponent>();
    OrderState::Finished
}

/// Advance an Unload order by one tick.
///
/// With a destination, a mobile transporter first walks into its `unload_range`
/// of it — an immobile one, or one whose walk can make no more progress, lets
/// its passengers out where it stands. Passengers then step out one per
/// `unload_period` ticks (a zero period empties the hold at once), each
/// revealed beside the transporter's footprint and sent on: to the
/// destination when the order named one — walking as close to it as the map
/// allows — otherwise to the rally point, if set. A blocked exit keeps the
/// order running and retries every tick; the order finishes when the hold is
/// empty.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut unload) = world.entity_mut(entity).take::<UnloadComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    if let Some(at) = unload.at
        && entity_def::of(world, entity).can_move()
    {
        let projection = world.resource::<Map>().projection();
        let (chaser_position, chaser_size) = entity_def::footprint(world, entity);
        match chase::advance(
            &mut unload.last_chase,
            projection,
            chaser_position,
            chaser_size,
            at,
            CellSize::ONE,
            entity_def::effective_stat_u32(world, entity, EntityStatId::UNLOAD_RANGE),
        ) {
            // The passengers can finish the trip on foot; the hold opens here.
            Destination::OutOfReach => {}
            Destination::Walk(move_order) => {
                world.entity_mut(entity).insert(unload);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }
    }

    if unload.cooldown > 0 {
        unload.cooldown -= 1;
        world.entity_mut(entity).insert(unload);
        return Processing::state(OrderState::InProcessing);
    }

    let period = entity_def::effective_stat_u32(world, entity, EntityStatId::UNLOAD_PERIOD);
    loop {
        let Some(passenger_id) = world
            .entity(entity)
            .get::<TransporterComponent>()
            .expect("a transporter holds a passenger list")
            .passengers
            .first()
            .copied()
        else {
            return Processing::state(OrderState::Finished);
        };

        let Some(passenger) = world.resource::<EntityIndex>().alive(passenger_id) else {
            // A passenger that died aboard left a stale entry; drop it and move on.
            remove_passenger(world, entity, passenger_id);
            continue;
        };

        let (origin, size) = entity_def::footprint(world, entity);
        if !spawn::reveal_entity_near(world, passenger, CellPos::from(origin), size) {
            // Boxed in: hold the door and retry next tick.
            world.entity_mut(entity).insert(unload);
            return Processing::state(OrderState::InProcessing);
        }

        remove_passenger(world, entity, passenger_id);
        world
            .entity_mut(passenger)
            .remove::<(BoardedComponent, GarrisonFireComponent)>();
        dispatch(world, entity, passenger, unload.at);

        if period > 0 {
            // One tick of the spacing elapses before the next process call
            // reads the counter, so the reveal lands exactly `period` later.
            unload.cooldown = period - 1;
            break;
        }
    }

    let emptied = world
        .entity(entity)
        .get::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
        .passengers
        .is_empty();
    if emptied {
        return Processing::state(OrderState::Finished);
    }
    world.entity_mut(entity).insert(unload);
    Processing::state(OrderState::InProcessing)
}

/// Drops `passenger_id` from the passenger list on `holder`.
fn remove_passenger(world: &mut World, holder: Entity, passenger_id: SimulationId) {
    world
        .entity_mut(holder)
        .get_mut::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
        .passengers
        .remove(&passenger_id);
}

/// Sends a freshly unloaded passenger on its way: to the order's destination if
/// it named one, otherwise wherever the transporter's rally point says.
fn dispatch(world: &mut World, holder: Entity, passenger: Entity, at: Option<FixedUVec2>) {
    match at {
        Some(target) => {
            if let Some(mut queue) = world.entity_mut(passenger).get_mut::<OrderQueueComponent>() {
                queue.push(
                    Order::Move {
                        target,
                        size: CellSize::ONE,
                        range: 0,
                    },
                    None,
                );
            }
        }
        None => rally::send(world, holder, passenger),
    }
}
