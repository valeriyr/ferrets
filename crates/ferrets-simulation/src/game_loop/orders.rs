//! Order lifecycle orchestration.
//!
//! Each tick the queue goes through two phases inside a single exclusive Bevy system:
//!
//! - [`prepare_tick`] — flush cancelled entries, then transition the front `New` order to
//!   `InProcessing`.
//! - [`process_tick`] — advance the front `InProcessing` order by one tick and handle
//!   `Finished` and `Suspended` transitions.
//!
//! Each order type module (`movement`, `attack`, …) implements `prepare`,
//! `prepare_suspended`, `cancel_processing`, and `process`. Each module owns its driver
//! component lifecycle — inserting and removing the component as part of those calls.
//! This module dispatches to the right implementation and enforces state-machine
//! invariants with assertions.

use bevy_ecs::{entity::Entity, world::World};

use super::movement;
use crate::{
    components::order_queue::{OrderQueueComponent, OrderState},
    order::Order,
};

/// Flush cancelled entries, then prepare the front `New` order.
///
/// The caller is responsible for taking `queue` out of the world before this call and
/// reinserting it after. This keeps the world borrow free so `movement::*` handlers can
/// access and mutate other components (e.g. `MoveComponent`, `LocationComponent`).
///
/// **Flush**: for every entry with a cancel policy, calls the per-type
/// `cancel_processing` (which handles driver removal). If the result is `Finished`,
/// removes the entry. Entries whose cancel returns `InProcessing` (Soft cancel accepted)
/// stay in the queue with their cancel policy cleared.
///
/// **Prepare loop**: while the front is `New`, calls the per-type `prepare` (which
/// handles driver insertion). If it returns `Finished` immediately, removes the entry
/// and loops to the next. If it returns `InProcessing`, sets the state and stops.
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

        let new_state = match &order {
            Order::Move { .. } => {
                movement::cancel_processing(entity, &order, policy, entry_state, world)
            }
        };

        debug_assert!(
            matches!(new_state, OrderState::InProcessing | OrderState::Finished),
            "cancel_processing must return InProcessing or Finished, got {:?}",
            new_state
        );

        if new_state == OrderState::Finished {
            queue.0.remove(i);
        } else {
            i += 1;
        }
    }

    // Prepare the front New entry.
    while let Some(front) = queue.front() {
        let state = front.state;
        let order = front.order.clone();

        debug_assert!(
            matches!(state, OrderState::New | OrderState::InProcessing),
            "front before prepare must be New or InProcessing, got {:?}",
            state
        );

        if state != OrderState::New {
            break;
        }

        let new_state = match &order {
            Order::Move { .. } => movement::prepare(entity, &order, world),
        };

        debug_assert!(
            matches!(new_state, OrderState::InProcessing | OrderState::Finished),
            "prepare must return InProcessing or Finished, got {:?}",
            new_state
        );

        if new_state == OrderState::Finished {
            queue.0.pop_front();
        } else {
            queue.front_mut().unwrap().state = OrderState::InProcessing;
            break;
        }
    }

    debug_assert!(
        queue
            .front()
            .is_none_or(|e| e.state == OrderState::InProcessing),
        "after prepare, front must be InProcessing or queue must be empty"
    );
}

/// Advance the front `InProcessing` order by one tick.
///
/// Dispatches to the per-type `process` implementation. Based on the result:
/// - `InProcessing`: nothing changes.
/// - `Finished`: pops the entry. The per-type handler is responsible for removing its
///   driver component (by not reinserting the taken component).
/// - `Suspended`: updates state. Suspended resume is handled by the next
///   [`prepare_tick`] call.
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

    let new_state = match &order {
        Order::Move { .. } => movement::process(entity, &order, world),
    };

    debug_assert!(
        matches!(
            new_state,
            OrderState::InProcessing | OrderState::Finished | OrderState::Suspended
        ),
        "process must return InProcessing, Finished, or Suspended, got {:?}",
        new_state
    );

    queue.0.front_mut().unwrap().state = new_state;

    if new_state == OrderState::Finished {
        queue.0.pop_front();
    }

    debug_assert!(
        queue.0.front().is_none_or(|e| matches!(
            e.state,
            OrderState::New | OrderState::InProcessing | OrderState::Suspended
        )),
        "after process, front must be New, InProcessing, Suspended, or queue must be empty"
    );
}
