//! Load order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    board,
    chase::{self, Destination},
    movement,
    orders::Processing,
};
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        transport::LoadComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    session::GameSession,
};
use ferrets_content::entity_stats::EntityStatId;

/// Called once when a Load order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when this entity would not take the target aboard — see
/// [`board::would_board`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target_id = order.load_target().expect("Load order must have a target");

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return OrderState::Finished;
    };
    if !board::would_board(world, target, entity) {
        return OrderState::Finished;
    }

    world
        .entity_mut(entity)
        .insert(LoadComponent::new(target_id));
    OrderState::InProcessing
}

/// Called when a Load order resumes from `Suspended` (its walk toward the
/// target just finished). The driver component survives suspension; validation
/// happens in [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Load entry that has a cancel policy.
///
/// Fetching stops immediately under both policies: nothing changed hands until
/// the walk arrived, and the transfer itself is atomic within a tick.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<LoadComponent>();
    OrderState::Finished
}

/// Advance a Load order by one tick.
///
/// Walk to within own `load_range` of the target (suspending on a chase move,
/// following it as it moves), then take it in: the target's own orders are
/// force-cancelled — a walk cut down mid-crossing settles onto its claimed
/// cell first — and it disappears aboard through the same transfer boarding
/// uses, cooldown included. Everything is re-checked at arrival; a target that
/// stopped qualifying on the way ends the order.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut load) = world.entity_mut(entity).take::<LoadComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let target_id = load.target;

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return Processing::state(OrderState::Finished);
    };
    if !board::would_board(world, target, entity) {
        return Processing::state(OrderState::Finished);
    }

    match chase::advance_to_entity(
        &mut load.last_chase,
        world,
        entity,
        target,
        entity_def::effective_stat_u32(world, entity, EntityStatId::LOAD_RANGE),
    ) {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            world.entity_mut(entity).insert(load);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }

    // Own boarding cooldown: a holder still admitting the last passenger keeps
    // the next one waiting where it stands.
    let tick = world.resource::<GameSession>().tick();
    let ready_at = board::boarding_ready_at(world, entity);
    if tick < ready_at {
        world.entity_mut(entity).insert(load);
        return Processing::state(OrderState::InProcessing);
    }

    // The target is going about its own business; stop it, and let a walk cut
    // down mid-crossing settle onto its claimed cell before it leaves the map —
    // its cancel runs in its own next prepare.
    if let Some(mut queue) = world.entity_mut(target).get_mut::<OrderQueueComponent>() {
        queue.cancel_all(CancelPolicy::Force);
    }
    if let MovementModel::Cell = world.resource::<Map>().movement_model()
        && movement::is_mid_crossing(entity_def::position(world, target))
    {
        world.entity_mut(entity).insert(load);
        return Processing::state(OrderState::InProcessing);
    }

    board::admit(world, entity, target);
    Processing::state(OrderState::Finished)
}
