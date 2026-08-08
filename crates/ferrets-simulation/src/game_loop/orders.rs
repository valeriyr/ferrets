//! Order lifecycle orchestration.
//!
//! Each tick the queue goes through three phases inside a single exclusive Bevy system:
//!
//! - [`prepare_tick`] — flush cancelled entries, then transition the front order to
//!   `InProcessing` (preparing `New` entries and resuming `Suspended` ones).
//! - [`watch_tick`] — let a suspended order interrupt the sub-order running in
//!   front of it, replacing the front.
//! - [`process_tick`] — advance the front `InProcessing` order by one tick and handle
//!   `Finished` and `Suspended` transitions.
//!
//! Each order type module (`movement`, `attack`, …) implements `prepare`,
//! `prepare_suspended`, `cancel_processing`, and `process`, plus an optional `watch`.
//! Each module owns its driver component lifecycle — inserting and removing the
//! component as part of those calls. This module dispatches to the right
//! implementation and enforces state-machine invariants with assertions.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    attack, attack_move, build, die, follow, guard, harvest, movement, patrol, repair, research,
    train,
};
use crate::{
    components::{
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
    },
    map::Map,
    movement_model::MovementModel,
    order::Order,
};

/// Result of advancing an order by one tick.
pub struct Processing {
    /// The order's new state.
    pub state: OrderState,
    /// A sub-order to execute at the front of the queue before this order resumes.
    /// Only valid together with [`OrderState::Suspended`].
    pub sub_order: Option<Order>,
}

impl Processing {
    /// A result with no sub-order.
    pub fn state(state: OrderState) -> Self {
        Self {
            state,
            sub_order: None,
        }
    }

    /// Suspends the order until `sub_order` finishes.
    pub fn suspend(sub_order: Order) -> Self {
        Self {
            state: OrderState::Suspended,
            sub_order: Some(sub_order),
        }
    }
}

fn dispatch_prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    match order {
        Order::Move { .. } => movement::prepare(entity, order, world),
        Order::Attack { .. } => attack::prepare(entity, order, world),
        Order::AttackMove { .. } => attack_move::prepare(entity, order, world),
        Order::Patrol { .. } => patrol::prepare(entity, order, world),
        Order::Guard { .. } => guard::prepare(entity, order, world),
        Order::Follow { .. } => follow::prepare(entity, order, world),
        Order::Train => train::prepare(entity, order, world),
        Order::Research { .. } => research::prepare(entity, order, world),
        Order::Build { .. } => build::prepare(entity, order, world),
        Order::Harvest { .. } => harvest::prepare(entity, order, world),
        Order::Repair { .. } => repair::prepare(entity, order, world),
        Order::Die => die::prepare(entity, order, world),
    }
}

fn dispatch_prepare_suspended(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    match order {
        Order::Attack { .. } => attack::prepare_suspended(entity, order, world),
        Order::AttackMove { .. } => attack_move::prepare_suspended(entity, order, world),
        Order::Patrol { .. } => patrol::prepare_suspended(entity, order, world),
        Order::Guard { .. } => guard::prepare_suspended(entity, order, world),
        Order::Follow { .. } => follow::prepare_suspended(entity, order, world),
        Order::Build { .. } => build::prepare_suspended(entity, order, world),
        Order::Harvest { .. } => harvest::prepare_suspended(entity, order, world),
        Order::Repair { .. } => repair::prepare_suspended(entity, order, world),
        Order::Move { .. } => unreachable!("Move orders never suspend"),
        Order::Train => unreachable!("Train orders never suspend"),
        Order::Research { .. } => unreachable!("Research orders never suspend"),
        Order::Die => unreachable!("Die orders never suspend"),
    }
}

fn dispatch_cancel(
    entity: Entity,
    order: &Order,
    policy: CancelPolicy,
    entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    match order {
        Order::Move { .. } => {
            movement::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Attack { .. } => {
            attack::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::AttackMove { .. } => {
            attack_move::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Patrol { .. } => {
            patrol::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Guard { .. } => guard::cancel_processing(entity, order, policy, entry_state, world),
        Order::Follow { .. } => {
            follow::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Train => train::cancel_processing(entity, order, policy, entry_state, world),
        Order::Research { .. } => {
            research::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Build { .. } => build::cancel_processing(entity, order, policy, entry_state, world),
        Order::Harvest { .. } => {
            harvest::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Repair { .. } => {
            repair::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Die => die::cancel_processing(entity, order, policy, entry_state, world),
    }
}

fn dispatch_process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    match order {
        Order::Move { .. } => Processing::state(movement::process(entity, order, world)),
        Order::Attack { .. } => attack::process(entity, order, world),
        Order::AttackMove { .. } => attack_move::process(entity, order, world),
        Order::Patrol { .. } => patrol::process(entity, order, world),
        Order::Guard { .. } => guard::process(entity, order, world),
        Order::Follow { .. } => follow::process(entity, order, world),
        Order::Train => Processing::state(train::process(entity, order, world)),
        Order::Research { .. } => Processing::state(research::process(entity, order, world)),
        Order::Build { .. } => build::process(entity, order, world),
        Order::Harvest { .. } => harvest::process(entity, order, world),
        Order::Repair { .. } => repair::process(entity, order, world),
        Order::Die => Processing::state(die::process(entity, order, world)),
    }
}

/// A suspended order watching the sub-order running in front of it. `Some`
/// interrupts: the sub-order is force-cancelled and replaced (see [`watch_tick`]).
fn dispatch_watch(
    entity: Entity,
    order: &Order,
    front: &Order,
    world: &mut World,
) -> Option<Order> {
    match order {
        Order::AttackMove { .. } => attack_move::watch(entity, order, front, world),
        Order::Guard { .. } => guard::watch(entity, order, front, world),
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::Patrol { .. }
        | Order::Follow { .. }
        | Order::Train
        | Order::Research { .. }
        | Order::Build { .. }
        | Order::Harvest { .. }
        | Order::Repair { .. }
        | Order::Die => None,
    }
}

/// Flush cancelled entries, then prepare the front order.
///
/// The caller is responsible for taking `queue` out of the world before this call and
/// reinserting it after. This keeps the world borrow free so per-type handlers can
/// access and mutate other components (e.g. `MoveComponent`, `LocationComponent`).
///
/// **Flush**: for every entry with a cancel policy, calls the per-type
/// `cancel_processing` (which handles driver removal). If the result is `Finished`,
/// removes the entry. Entries whose cancel returns `InProcessing` (cancel deferred or
/// refused) stay in the queue with their cancel policy cleared.
///
/// **Prepare loop**: while the front is `New` or `Suspended`, calls the per-type
/// `prepare` or `prepare_suspended` (which handle driver insertion). If it returns
/// `Finished` immediately, removes the entry and loops to the next. If it returns
/// `InProcessing`, sets the state and stops.
///
/// After this call the front entry (if any) is always `InProcessing`.
pub fn prepare_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    // Flush cancelled entries.
    let mut i = 0;
    while let Some(e) = queue.0.get(i) {
        let Some(policy) = e.cancel else {
            i += 1;
            continue;
        };
        let order = e.order.clone();
        let entry_state = e.state;

        // Clear cancel flag before dispatching — prevents re-cancellation on the next tick.
        queue.0[i].cancel = None;

        let new_state = dispatch_cancel(entity, &order, policy, entry_state, world);

        match new_state {
            OrderState::Finished => {
                queue.0.remove(i);
            }
            OrderState::InProcessing => i += 1,
            OrderState::New | OrderState::Suspended => unreachable!(
                "cancel_processing must return InProcessing or Finished, got {:?}",
                new_state
            ),
        }
    }

    prepare_front(entity, queue, world);
}

/// Give the entry directly behind the front — when it is `Suspended`, i.e. the
/// front is a sub-order it spawned — a chance to interrupt that sub-order.
///
/// On an interrupt, the front is force-cancelled, the replacement pushed in
/// its place, and the prepare loop re-run so the front is `InProcessing` again
/// for [`process_tick`]. The watcher itself stays suspended.
///
/// Watchers are only consulted while the entity rests on a cell: interrupting
/// a `Move` mid-crossing would leave a fractional position with no crossing
/// state to finish it.
pub fn watch_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    let Some(watcher) = queue.0.get(1) else {
        return;
    };
    if watcher.state != OrderState::Suspended {
        return;
    }
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;
    match world.resource::<Map>().movement_model() {
        // A continuous mover's position is a free point with no crossing
        // state to finish — watchers may always interrupt.
        MovementModel::Continuous => {}
        MovementModel::Cell => {
            if movement::is_mid_crossing(position) {
                return;
            }
        }
    }

    let watcher_order = watcher.order.clone();
    let front_order = queue.front().unwrap().order.clone();
    let Some(replacement) = dispatch_watch(entity, &watcher_order, &front_order, world) else {
        return;
    };

    let front_state = queue.front().unwrap().state;
    let cancelled = dispatch_cancel(
        entity,
        &front_order,
        CancelPolicy::Force,
        front_state,
        world,
    );
    debug_assert_eq!(
        cancelled,
        OrderState::Finished,
        "a force-cancelled sub-order must stop immediately"
    );
    queue.0.pop_front();
    queue.push_front(replacement);

    prepare_front(entity, queue, world);
}

/// Advance the front `InProcessing` order by one tick.
///
/// Dispatches to the per-type `process` implementation. Based on the result:
/// - `InProcessing`: nothing changes.
/// - `Finished`: pops the entry. The per-type handler is responsible for removing its
///   driver component (by not reinserting the taken component).
/// - `Suspended`: updates state and pushes the requested sub-order to the front.
///   The suspended entry resumes via the next [`prepare_tick`] call after the
///   sub-order finishes.
///
/// After this call the front entry (if any) is `New`, `InProcessing`, or `Suspended`.
pub fn process_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    let Some(front) = queue.0.front() else { return };
    debug_assert_eq!(
        front.state,
        OrderState::InProcessing,
        "process_tick requires an InProcessing front entry"
    );

    let order = front.order.clone();

    let result = dispatch_process(entity, &order, world);

    debug_assert!(
        result.sub_order.is_none() || result.state == OrderState::Suspended,
        "a sub-order is only valid together with Suspended"
    );

    queue.0.front_mut().unwrap().state = result.state;

    match result.state {
        OrderState::Finished => {
            queue.0.pop_front();
        }
        OrderState::InProcessing | OrderState::Suspended => {}
        OrderState::New => unreachable!(
            "process must return InProcessing, Finished, or Suspended, got {:?}",
            result.state
        ),
    }

    if let Some(sub_order) = result.sub_order {
        queue.push_front(sub_order);
    }

    debug_assert!(
        queue.0.front().is_none_or(|e| matches!(
            e.state,
            OrderState::New | OrderState::InProcessing | OrderState::Suspended
        )),
        "after process, front must be New, InProcessing, Suspended, or queue must be empty"
    );
}

/// Prepare the front entry until it is `InProcessing` or the queue is empty.
pub(crate) fn prepare_front(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    while let Some(front) = queue.front() {
        let state = front.state;
        let order = front.order.clone();

        debug_assert!(
            !matches!(state, OrderState::Finished),
            "Finished entries must never stay in the queue"
        );

        let new_state = match state {
            OrderState::InProcessing => break,
            OrderState::New => dispatch_prepare(entity, &order, world),
            OrderState::Suspended => dispatch_prepare_suspended(entity, &order, world),
            OrderState::Finished => unreachable!(),
        };

        match new_state {
            OrderState::Finished => {
                queue.0.pop_front();
            }
            OrderState::InProcessing => {
                queue.front_mut().unwrap().state = OrderState::InProcessing;
                break;
            }
            OrderState::New | OrderState::Suspended => unreachable!(
                "prepare must return InProcessing or Finished, got {:?}",
                new_state
            ),
        }
    }

    debug_assert!(
        queue
            .front()
            .is_none_or(|e| e.state == OrderState::InProcessing),
        "after prepare, front must be InProcessing or queue must be empty"
    );
}
