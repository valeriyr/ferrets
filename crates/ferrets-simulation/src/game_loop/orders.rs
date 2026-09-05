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
//! Each order type module (`movement`, `attack`, …) implements `can_start`,
//! `prepare`, `prepare_suspended`, `cancel_processing`, and `process`, plus an
//! optional `watch`. `can_start` is the one start check for its order, run
//! before the order is pushed and again by `prepare` when it reaches the
//! front.
//! Each module owns its driver component lifecycle — inserting and removing the
//! component as part of those calls. This module dispatches to the right
//! implementation and enforces state-machine invariants with assertions.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    attack, attack_move, board, build, die, follow, guard, harvest, load, morph, movement, patrol,
    repair, research, train, unload,
};
use crate::{
    components::{
        build::BuildComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
    },
    entity_def::{self, Operation},
    map::Map,
    movement_model::MovementModel,
    order::Order,
};

/// Why an entity may not start an order now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The type cannot do this at all.
    Incapable,
    /// The entity is still being raised.
    UnderConstruction,
    /// A field switches the entity off.
    Disabled,
    /// The order has no work in it: nobody aboard, nothing to mend.
    NothingToDo,
    /// The named target is gone or out of sight.
    TargetGone,
    /// The target exists but is not one this order takes.
    TargetUnfit,
}

/// What an order does while its entity is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisabledConduct {
    /// Stays where it is, untouched, until the entity operates again.
    Holds,
    /// Keeps running.
    Completes,
    /// Force-cancelled.
    Cancels,
}

/// What the order loop does to the queue once an order's tick is over, on the
/// order's instruction — the queue is the loop's to hold while an order runs.
pub enum FollowUp {
    /// Nothing beyond recording the new state.
    None,
    /// Push this sub-order to the front, to run before the order resumes. Only
    /// valid together with [`OrderState::Suspended`].
    SubOrder(Order),
    /// The entity died during the tick: cancel whatever else is queued and push
    /// its Die order, as a death from outside an order would have.
    Dies,
}

/// Result of advancing an order by one tick.
pub struct Processing {
    /// The order's new state.
    pub state: OrderState,
    /// What the loop does to the queue afterwards.
    pub follow_up: FollowUp,
}

impl Processing {
    /// A result with no follow-up.
    pub fn state(state: OrderState) -> Self {
        Self {
            state,
            follow_up: FollowUp::None,
        }
    }

    /// Suspends the order until `sub_order` finishes.
    pub fn suspend(sub_order: Order) -> Self {
        Self {
            state: OrderState::Suspended,
            follow_up: FollowUp::SubOrder(sub_order),
        }
    }

    /// Finishes the order with the entity dead: it was despawned during the
    /// tick, and the loop gives it the Die order its queue was out of reach for.
    pub fn finished_dying() -> Self {
        Self {
            state: OrderState::Finished,
            follow_up: FollowUp::Dies,
        }
    }
}

/// Whether `entity` may start `order` now: its type has the capability, its
/// [`Operation`] admits the order, and the order's target is there and fit.
/// Costs, supply and requirements are not judged here.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    match order {
        Order::Move { .. } => movement::can_start(world, entity, order),
        Order::Attack { .. } => attack::can_start(world, entity, order),
        Order::AttackMove { .. } => attack_move::can_start(world, entity, order),
        Order::Patrol { .. } => patrol::can_start(world, entity, order),
        Order::Guard { .. } => guard::can_start(world, entity, order),
        Order::Follow { .. } => follow::can_start(world, entity, order),
        Order::Train => train::can_start(world, entity, order),
        Order::Research { .. } => research::can_start(world, entity, order),
        Order::Morph { .. } => morph::can_start(world, entity, order),
        Order::Build { .. } => build::can_start(world, entity, order),
        Order::Harvest { .. } => harvest::can_start(world, entity, order),
        Order::Repair { .. } => repair::can_start(world, entity, order),
        Order::Board { .. } => board::can_start(world, entity, order),
        Order::Load { .. } => load::can_start(world, entity, order),
        Order::Unload { .. } => unload::can_start(world, entity, order),
        Order::Die => die::can_start(world, entity, order),
    }
}

/// The refusal an entity's [`Operation`] hands an order that only an operating
/// entity may run.
pub(super) fn requires_operating(world: &World, entity: Entity) -> Result<(), Refusal> {
    match entity_def::operation(world, entity) {
        Operation::Operating => Ok(()),
        Operation::UnderConstruction => Err(Refusal::UnderConstruction),
        Operation::Disabled => Err(Refusal::Disabled),
    }
}

/// The refusal an entity's [`Operation`] hands an order that a disabled
/// entity may still queue and wait with.
pub(super) fn requires_raised(world: &World, entity: Entity) -> Result<(), Refusal> {
    match entity_def::operation(world, entity) {
        Operation::Operating | Operation::Disabled => Ok(()),
        Operation::UnderConstruction => Err(Refusal::UnderConstruction),
    }
}

/// The refusal a target's [`Operation`] hands an order that needs it
/// operating.
pub(super) fn target_operating(world: &World, target: Entity) -> Result<(), Refusal> {
    match entity_def::operation(world, target) {
        Operation::Operating => Ok(()),
        Operation::UnderConstruction | Operation::Disabled => Err(Refusal::TargetUnfit),
    }
}

/// The [`DisabledConduct`] of `entity`'s `order`, an entry in `state`.
fn disabled_conduct(
    world: &World,
    entity: Entity,
    order: &Order,
    state: OrderState,
) -> DisabledConduct {
    match order {
        Order::Train | Order::Research { .. } => DisabledConduct::Holds,
        Order::Die => DisabledConduct::Completes,
        // A change under way lands; one still queued has nothing to finish.
        Order::Morph { .. } => match state {
            OrderState::InProcessing | OrderState::Suspended => DisabledConduct::Completes,
            OrderState::New | OrderState::Finished => DisabledConduct::Cancels,
        },
        // A Build with its site raised finishes the site; one still queued or
        // walking to the ground has raised nothing to finish.
        Order::Build { .. } => match state {
            OrderState::InProcessing | OrderState::Suspended => match world
                .entity(entity)
                .get::<BuildComponent>()
                .and_then(|build| build.building)
            {
                Some(_) => DisabledConduct::Completes,
                None => DisabledConduct::Cancels,
            },
            OrderState::New | OrderState::Finished => DisabledConduct::Cancels,
        },
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::AttackMove { .. }
        | Order::Patrol { .. }
        | Order::Guard { .. }
        | Order::Follow { .. }
        | Order::Harvest { .. }
        | Order::Repair { .. }
        | Order::Board { .. }
        | Order::Load { .. }
        | Order::Unload { .. } => DisabledConduct::Cancels,
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
        Order::Morph { .. } => morph::prepare(entity, order, world),
        Order::Build { .. } => build::prepare(entity, order, world),
        Order::Harvest { .. } => harvest::prepare(entity, order, world),
        Order::Repair { .. } => repair::prepare(entity, order, world),
        Order::Board { .. } => board::prepare(entity, order, world),
        Order::Load { .. } => load::prepare(entity, order, world),
        Order::Unload { .. } => unload::prepare(entity, order, world),
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
        Order::Board { .. } => board::prepare_suspended(entity, order, world),
        Order::Load { .. } => load::prepare_suspended(entity, order, world),
        Order::Unload { .. } => unload::prepare_suspended(entity, order, world),
        Order::Move { .. } => unreachable!("Move orders never suspend"),
        Order::Train => unreachable!("Train orders never suspend"),
        Order::Research { .. } => unreachable!("Research orders never suspend"),
        Order::Morph { .. } => unreachable!("Morph orders never suspend"),
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
        Order::Morph { .. } => morph::cancel_processing(entity, order, policy, entry_state, world),
        Order::Build { .. } => build::cancel_processing(entity, order, policy, entry_state, world),
        Order::Harvest { .. } => {
            harvest::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Repair { .. } => {
            repair::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Board { .. } => board::cancel_processing(entity, order, policy, entry_state, world),
        Order::Load { .. } => load::cancel_processing(entity, order, policy, entry_state, world),
        Order::Unload { .. } => {
            unload::cancel_processing(entity, order, policy, entry_state, world)
        }
        Order::Die => die::cancel_processing(entity, order, policy, entry_state, world),
    }
}

fn dispatch_process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    match order {
        Order::Move { .. } => movement::process(entity, order, world),
        Order::Attack { .. } => attack::process(entity, order, world),
        Order::AttackMove { .. } => attack_move::process(entity, order, world),
        Order::Patrol { .. } => patrol::process(entity, order, world),
        Order::Guard { .. } => guard::process(entity, order, world),
        Order::Follow { .. } => follow::process(entity, order, world),
        Order::Train => train::process(entity, order, world),
        Order::Research { .. } => research::process(entity, order, world),
        Order::Morph { .. } => morph::process(entity, order, world),
        Order::Build { .. } => build::process(entity, order, world),
        Order::Harvest { .. } => harvest::process(entity, order, world),
        Order::Repair { .. } => repair::process(entity, order, world),
        Order::Board { .. } => board::process(entity, order, world),
        Order::Load { .. } => load::process(entity, order, world),
        Order::Unload { .. } => unload::process(entity, order, world),
        Order::Die => die::process(entity, order, world),
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
        | Order::Morph { .. }
        | Order::Build { .. }
        | Order::Harvest { .. }
        | Order::Repair { .. }
        | Order::Board { .. }
        | Order::Load { .. }
        | Order::Unload { .. }
        | Order::Die => None,
    }
}

/// Flush cancelled entries, then prepare the front order.
///
/// The caller is responsible for taking `queue` out of the world before this call and
/// reinserting it after. This keeps the world borrow free so per-type handlers can
/// access and mutate other components (e.g. `MoveComponent`, `LocationComponent`).
///
/// **Sweep**: a disabled entity first marks every entry whose disabled
/// conduct is `Cancels` for a force cancel, so the flush below
/// empties them in the same tick.
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
    match entity_def::operation(world, entity) {
        Operation::Disabled => {
            for entry in &mut queue.0 {
                match disabled_conduct(world, entity, &entry.order, entry.state) {
                    DisabledConduct::Cancels => entry.cancel = Some(CancelPolicy::Force),
                    DisabledConduct::Holds | DisabledConduct::Completes => {}
                }
            }
        }
        Operation::Operating | Operation::UnderConstruction => {}
    }

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
    let position = entity_def::position(world, entity);
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
/// - A `Dies` follow-up: the entity was despawned during the tick; the rest of
///   the queue is marked for a force cancel and its Die order pushed.
///
/// A disabled entity's front order is skipped without dispatch when its
/// disabled conduct is `Holds`; it resumes untouched once the entity
/// operates again.
///
/// After this call the front entry (if any) is `New`, `InProcessing`, or `Suspended`.
pub fn process_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    let Some(front) = queue.0.front() else { return };
    match (
        entity_def::operation(world, entity),
        disabled_conduct(world, entity, &front.order, front.state),
    ) {
        (Operation::Disabled, DisabledConduct::Holds) => return,
        (Operation::Disabled, DisabledConduct::Completes | DisabledConduct::Cancels)
        | (Operation::Operating | Operation::UnderConstruction, _) => {}
    }
    // An order pushed onto this queue after its prepare ran — by another
    // entity's processing, e.g. a transporter dispatching a passenger it just
    // let out — waits for the next tick's prepare.
    if front.state == OrderState::New {
        return;
    }
    debug_assert_eq!(
        front.state,
        OrderState::InProcessing,
        "process_tick requires an InProcessing front entry"
    );

    let order = front.order.clone();

    let result = dispatch_process(entity, &order, world);

    debug_assert!(
        match result.follow_up {
            FollowUp::SubOrder(_) => result.state == OrderState::Suspended,
            FollowUp::Dies => result.state == OrderState::Finished,
            FollowUp::None => true,
        },
        "a sub-order goes with Suspended and a death with Finished"
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

    match result.follow_up {
        FollowUp::None => {}
        FollowUp::SubOrder(sub_order) => queue.push_front(sub_order),
        FollowUp::Dies => queue.push(Order::Die, Some(CancelPolicy::Force)),
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
