//! Patrol order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::orders::{self, Processing, Refusal};
use crate::{
    components::{
        order_queue::{CancelPolicy, OrderState},
        patrol::PatrolComponent,
    },
    entity_def,
    order::Order,
};

/// Whether `entity` may start a Patrol: its type moves and it operates.
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    if !entity_def::of(world, entity).can_move() {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)
}

/// Called once when a Patrol order becomes the front `New` entry.
///
/// Records the current position as the return endpoint and returns
/// `InProcessing`, or `Finished` immediately when the order cannot start — see
/// [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    let home = entity_def::position(world, entity);
    world.entity_mut(entity).insert(PatrolComponent {
        home,
        outbound: true,
    });
    OrderState::InProcessing
}

/// Called when a Patrol order resumes from `Suspended` (a leg just finished).
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Patrol entry that has a cancel policy.
///
/// A patrol stops immediately under both policies.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> Processing {
    world.entity_mut(entity).remove::<PatrolComponent>();
    Processing::state(OrderState::Finished)
}

/// Whether a Patrol can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance a Patrol order by one tick: suspend into the next attack-move leg,
/// alternating between the order's target and the recorded home endpoint.
/// Engaging and resuming live entirely in the leg; the patrol itself never
/// finishes on its own.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target = order
        .patrol_target()
        .expect("Patrol order must have a target");

    let Some((outbound, home)) = world
        .entity(entity)
        .get::<PatrolComponent>()
        .map(|driver| (driver.outbound, driver.home))
    else {
        return Processing::state(OrderState::Finished);
    };

    let leg = Order::AttackMove {
        target: if outbound { target } else { home },
    };
    // The leg turns only once its walk can start: a patroller that cannot
    // walk right now keeps facing the same way and tries the same leg again.
    if orders::can_start(world, entity, &leg).is_err() {
        return Processing::state(OrderState::InProcessing);
    }
    world
        .entity_mut(entity)
        .get_mut::<PatrolComponent>()
        .expect("a patroller carries its driver")
        .outbound = !outbound;
    Processing::suspend(leg)
}
