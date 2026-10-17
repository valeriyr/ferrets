//! Order lifecycle orchestration.
//!
//! Each tick the queue goes through three phases inside a single exclusive Bevy system:
//!
//! - [`prepare_tick`] — flush canceled entries, then transition the front order to
//!   `InProcessing` (preparing `New` entries and resuming `Suspended` ones).
//! - [`watch_tick`] — let a suspended order interrupt the sub-order running in
//!   front of it, replacing the front.
//! - [`process_tick`] — advance the front `InProcessing` order by one tick and handle
//!   `Finished` and `Suspended` transitions.
//!
//! Each order type module (`movement`, `attack`, …) implements `can_start`,
//! `prepare`, `prepare_suspended`, `cancel_processing`, `survives_soft_cancel`,
//! and `process`, plus an optional `watch`. `can_start` is the one start check for its order, run
//! before the order is pushed and again by `prepare` when it reaches the
//! front.
//! Each module owns its driver component lifecycle — inserting and removing the
//! component as part of those calls. This module dispatches to the right
//! implementation and enforces state-machine invariants with assertions.
//!
//! A processor resolves any other entity whose position it reads through
//! [`EntityIndex::interactable`](crate::entity_index::EntityIndex::interactable)
//! on every tick it reads it: a target that went off the map is lost to the
//! order. `alive` and `any` are for lists, owners and events. A processor reads
//! its own body's position only in the order that took the body off the map,
//! or once its conduct lets it run standing.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    attack, attack_move, board, build, cast, die, follow, guard, harvest, load, morph, movement,
    patrol, repair, research, train, unload,
};
use crate::{
    components::{
        build::BuildComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, IdlenessComponent, OrderQueueComponent, OrderState},
    },
    entity_def::{self, Operation, Outage},
    entity_index::EntityIndex,
    map::Map,
    movement_model::{self, MovementModel},
    order::Order,
    session::GameSession,
};

/// Why an entity may not start an order now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The type cannot do this at all.
    Incapable,
    /// The entity is still being raised.
    UnderConstruction,
    /// The entity is switched off: a field it needs is gone, or it is an
    /// annex standing with no primary on terms that stop it working.
    Disabled,
    /// The entity is off the map, waiting on a free cell to come back to.
    Stranded,
    /// The order has no work in it: nobody aboard, nothing to mend.
    NothingToDo,
    /// The named target is gone or out of sight.
    TargetGone,
    /// The target exists but is not one this order takes.
    TargetUnfit,
    /// The entity's queue holds work a soft flush would leave in place, it is
    /// changing form, it sits in a berth, or workers sit in its berths.
    Busy,
    /// Nothing around the entity can take what the order stands on the ground.
    NoRoom,
    /// The player's supply has no room for what the order makes.
    NoSupply,
}

/// What an order does while its entity is in a given [`Operation`], or off
/// the map.
#[derive(Debug)]
enum Conduct {
    /// Stays where it is, untouched — neither started nor advanced — until the
    /// entity operates again and stands on the map.
    Holds,
    /// Keeps running.
    Completes,
    /// Ends at once, keeping nothing back.
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
    /// The order finished and whatever else is queued is canceled with this
    /// policy.
    Cancels(CancelPolicy),
}

/// Result of an order's step: a tick advanced, or a cancel judged.
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

    /// Finishes the order and cancels the rest of the queue with `policy`.
    pub fn finished_canceling(policy: CancelPolicy) -> Self {
        Self {
            state: OrderState::Finished,
            follow_up: FollowUp::Cancels(policy),
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
        Order::Cast { .. } => cast::can_start(world, entity, order),
        Order::Die => die::can_start(world, entity, order),
    }
}

/// Whether `entity`'s queue holds an order of a kind a soft cancel can leave
/// in place — paid production, a change of form, a death — so a command that
/// must find the entity idle is refused.
pub(super) fn resists_soft_flush(world: &World, entity: Entity) -> bool {
    world
        .entity(entity)
        .get::<OrderQueueComponent>()
        .is_some_and(|queue| {
            queue
                .0
                .iter()
                .any(|entry| survives_soft_cancel(&entry.order))
        })
}

/// Whether orders of this kind can stand through a soft cancel — the answer
/// the kind's own module gives, the one its `cancel_processing` acts on.
fn survives_soft_cancel(order: &Order) -> bool {
    match order {
        Order::Move { .. } => movement::survives_soft_cancel(),
        Order::Attack { .. } => attack::survives_soft_cancel(),
        Order::AttackMove { .. } => attack_move::survives_soft_cancel(),
        Order::Patrol { .. } => patrol::survives_soft_cancel(),
        Order::Guard { .. } => guard::survives_soft_cancel(),
        Order::Follow { .. } => follow::survives_soft_cancel(),
        Order::Train => train::survives_soft_cancel(),
        Order::Research { .. } => research::survives_soft_cancel(),
        Order::Morph { .. } => morph::survives_soft_cancel(),
        Order::Build { .. } => build::survives_soft_cancel(),
        Order::Harvest { .. } => harvest::survives_soft_cancel(),
        Order::Repair { .. } => repair::survives_soft_cancel(),
        Order::Board { .. } => board::survives_soft_cancel(),
        Order::Load { .. } => load::survives_soft_cancel(),
        Order::Unload { .. } => unload::survives_soft_cancel(),
        Order::Cast { .. } => cast::survives_soft_cancel(),
        Order::Die => die::survives_soft_cancel(),
    }
}

/// The refusal an entity's state hands work that queues and waits: busy while
/// a change of form is under way, refused while still being raised or standing
/// as an annex with no primary, and admitted when a field or a buff switched it
/// off — the work waits for the outage to pass.
pub(super) fn admits_queued_work(world: &World, entity: Entity) -> Result<(), Refusal> {
    // A change under way makes it busy whatever else it is: the form it lands
    // as may have no queue to hold the work.
    if entity_def::changing(world, entity) {
        return Err(Refusal::Busy);
    }
    match entity_def::operation(world, entity) {
        Operation::Operating => Ok(()),
        // An annex with nothing docked takes no new work at all; a building a
        // field or a buff switched off queues it and waits for the outage to
        // pass.
        Operation::Disabled(Outage::Orphaned) => Err(Refusal::Disabled),
        Operation::Disabled(Outage::Field | Outage::Buff) => Ok(()),
        Operation::UnderConstruction => Err(Refusal::UnderConstruction),
        // Off the map waiting on a cell, it takes no new work.
        Operation::Stranded => Err(Refusal::Stranded),
    }
}

/// The refusal an entity's [`Operation`] hands an order that only an operating
/// entity may run.
pub(super) fn requires_operating(world: &World, entity: Entity) -> Result<(), Refusal> {
    match entity_def::operation(world, entity) {
        Operation::Operating => Ok(()),
        Operation::UnderConstruction => Err(Refusal::UnderConstruction),
        Operation::Disabled(_) => Err(Refusal::Disabled),
        Operation::Stranded => Err(Refusal::Stranded),
    }
}

/// The refusal a target's [`Operation`] hands an order that needs it
/// operating.
pub(super) fn target_operating(world: &World, target: Entity) -> Result<(), Refusal> {
    match entity_def::operation(world, target) {
        Operation::Operating => Ok(()),
        Operation::UnderConstruction | Operation::Disabled(_) | Operation::Stranded => {
            Err(Refusal::TargetUnfit)
        }
    }
}

/// The [`Conduct`] of `entity`'s `order`, an entry in `state`, while the
/// entity is in `operation`.
fn conduct(
    world: &World,
    entity: Entity,
    operation: Operation,
    order: &Order,
    state: OrderState,
) -> Conduct {
    // Off the map, only the order that took the entity there runs, and a
    // death; everything else waits for it to stand again.
    if entity_def::hidden(world, entity) {
        match order {
            Order::Die => {}
            Order::Harvest { .. } | Order::Build { .. } | Order::Repair { .. } => match state {
                OrderState::InProcessing | OrderState::Suspended => {}
                OrderState::New | OrderState::Finished => return Conduct::Holds,
            },
            Order::Train
            | Order::Research { .. }
            | Order::Morph { .. }
            | Order::Move { .. }
            | Order::Attack { .. }
            | Order::AttackMove { .. }
            | Order::Patrol { .. }
            | Order::Guard { .. }
            | Order::Follow { .. }
            | Order::Board { .. }
            | Order::Load { .. }
            | Order::Unload { .. }
            | Order::Cast { .. } => return Conduct::Holds,
        }
    }
    match operation {
        Operation::Operating | Operation::UnderConstruction => Conduct::Completes,
        // Off the map waiting on a cell: the work that stranded the body waits
        // for it to stand, a death does not.
        Operation::Stranded => match order {
            Order::Die => Conduct::Completes,
            Order::Build { .. } | Order::Harvest { .. } | Order::Repair { .. } => Conduct::Holds,
            Order::Train
            | Order::Research { .. }
            | Order::Morph { .. }
            | Order::Move { .. }
            | Order::Attack { .. }
            | Order::AttackMove { .. }
            | Order::Patrol { .. }
            | Order::Guard { .. }
            | Order::Follow { .. }
            | Order::Board { .. }
            | Order::Load { .. }
            | Order::Unload { .. }
            | Order::Cast { .. } => unreachable!("the hidden rule holds it off the map"),
        },
        Operation::Disabled(_) => match order {
            Order::Train | Order::Research { .. } => Conduct::Holds,
            Order::Die => Conduct::Completes,
            // A change under way lands; one still queued has nothing to finish.
            Order::Morph { .. } => match state {
                OrderState::InProcessing | OrderState::Suspended => Conduct::Completes,
                OrderState::New | OrderState::Finished => Conduct::Cancels,
            },
            // A Build with its site raised finishes the site; one still queued
            // or walking to the ground has raised nothing to finish.
            Order::Build { .. } => match state {
                OrderState::InProcessing | OrderState::Suspended => match world
                    .entity(entity)
                    .get::<BuildComponent>()
                    .and_then(|build| build.building)
                {
                    Some(_) => Conduct::Completes,
                    None => Conduct::Cancels,
                },
                OrderState::New | OrderState::Finished => Conduct::Cancels,
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
            | Order::Unload { .. }
            | Order::Cast { .. } => Conduct::Cancels,
        },
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
        Order::Cast { .. } => cast::prepare(entity, order, world),
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
        Order::Cast { .. } => cast::prepare_suspended(entity, order, world),
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
) -> Processing {
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
        Order::Cast { .. } => cast::cancel_processing(entity, order, policy, entry_state, world),
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
        Order::Cast { .. } => cast::process(entity, order, world),
        Order::Die => die::process(entity, order, world),
    }
}

/// A suspended order watching the sub-order running in front of it. `Some`
/// interrupts: the sub-order is force-canceled and replaced (see [`watch_tick`]).
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
        | Order::Cast { .. }
        | Order::Die => None,
    }
}

/// Flush canceled entries, then prepare the front order.
///
/// The caller is responsible for taking `queue` out of the world before this call and
/// reinserting it after. This keeps the world borrow free so per-type handlers can
/// access and mutate other components (e.g. `MoveComponent`, `LocationComponent`).
///
/// **Sweep**: an entity disabled or stranded first marks every entry whose
/// conduct is `Cancels` for a force cancel, so the flush below empties them in
/// the same tick.
///
/// **Flush**: for every entry with a cancel policy, calls the per-type
/// `cancel_processing` (which handles driver removal). If the result is `Finished`,
/// removes the entry. Entries whose cancel returns `InProcessing` (deferred or
/// refused) stay in the queue with their cancel policy cleared.
///
/// **Prepare loop**: while the front is `New` or `Suspended`, calls the per-type
/// `prepare` or `prepare_suspended` (which handle driver insertion). If it returns
/// `Finished` immediately, removes the entry and loops to the next. If it returns
/// `InProcessing`, sets the state and stops.
///
/// After this call the front entry (if any) is `InProcessing`, or held by its
/// conduct.
pub fn prepare_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    let operation = entity_def::operation(world, entity);
    match operation {
        Operation::Operating | Operation::UnderConstruction => {}
        Operation::Disabled(_) | Operation::Stranded => {
            for entry in &mut queue.0 {
                match conduct(world, entity, operation, &entry.order, entry.state) {
                    Conduct::Cancels => entry.cancel = Some(CancelPolicy::Force),
                    Conduct::Holds | Conduct::Completes => {}
                }
            }
        }
    }

    // A flushed cancel can take the entity off the map or set it down, so the
    // front is prepared against what the entity is after it.
    let operation = match flush_canceled(entity, queue, world) {
        Flushed::Nothing => operation,
        Flushed::Entries => entity_def::operation(world, entity),
    };
    prepare_front(entity, operation, queue, world);
}

/// Cancels every order `entity` holds with `policy` and applies the cancels at
/// once, with what each order took left spent. Under a force only a death
/// survives: a `Die` already queued, or one a canceled change of form hands
/// on, keeps running. Under a soft cancel the kinds that stand through one —
/// production, a committed change of form, a death — answer for themselves,
/// and every other order is forced.
///
/// How something outside the entity's own orders ends them; an order ending
/// the rest of its own entity's queue returns [`FollowUp::Cancels`] instead.
pub fn drop_all(world: &mut World, entity: Entity, policy: CancelPolicy) {
    let mut queue = world
        .entity_mut(entity)
        .take::<OrderQueueComponent>()
        .expect("simulation entities carry an order queue");
    match policy {
        CancelPolicy::Force => queue.cancel_all(CancelPolicy::Force),
        CancelPolicy::Soft => {
            for entry in &mut queue.0 {
                let entry_policy = if survives_soft_cancel(&entry.order) {
                    CancelPolicy::Soft
                } else {
                    CancelPolicy::Force
                };
                entry.cancel = Some(
                    entry
                        .cancel
                        .map_or(entry_policy, |marked| marked.stronger(entry_policy)),
                );
            }
        }
    }
    flush_canceled(entity, &mut queue, world);
    match policy {
        CancelPolicy::Force => assert!(
            queue
                .0
                .iter()
                .all(|entry| matches!(entry.order, Order::Die)),
            "a force cancel finishes every order but a death"
        ),
        CancelPolicy::Soft => assert!(
            queue
                .0
                .iter()
                .all(|entry| survives_soft_cancel(&entry.order)),
            "a soft drop finishes every order of a kind that does not stand"
        ),
    }
    world.entity_mut(entity).insert(queue);
}

/// What a flush of the canceled entries did.
#[derive(Debug)]
enum Flushed {
    /// No entry was marked; the world is as it was.
    Nothing,
    /// At least one cancel was dispatched.
    Entries,
}

/// Applies every pending cancel in `queue`: each flagged entry is dispatched
/// to its processor's cancel and removed once it reports `Finished`.
fn flush_canceled(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) -> Flushed {
    let mut flushed = Flushed::Nothing;
    let mut i = 0;
    while let Some(e) = queue.0.get(i) {
        let Some(policy) = e.cancel else {
            i += 1;
            continue;
        };
        let order = e.order.clone();
        let entry_state = e.state;

        // Clear the flag before dispatching — prevents re-cancellation on the next tick.
        queue.0[i].cancel = None;
        flushed = Flushed::Entries;

        let result = dispatch_cancel(entity, &order, policy, entry_state, world);

        match result.state {
            OrderState::Finished => {
                queue.0.remove(i);
            }
            OrderState::InProcessing => i += 1,
            OrderState::New | OrderState::Suspended => unreachable!(
                "cancel_processing must return InProcessing or Finished, got {:?}",
                result.state
            ),
        }
        match result.follow_up {
            FollowUp::None => {}
            FollowUp::Dies => queue.push(Order::Die, Some(CancelPolicy::Force)),
            FollowUp::SubOrder(_) => unreachable!("a cancel never suspends into a sub-order"),
            FollowUp::Cancels(_) => unreachable!("a cancel cancels nothing beyond itself"),
        }
    }
    flushed
}

/// Give the entry directly behind the front — when it is `Suspended`, i.e. the
/// front is a sub-order it spawned — a chance to interrupt that sub-order.
///
/// On an interrupt, the front is force-canceled, the replacement pushed in
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
    // Off the map no watcher interrupts anything: the front holds untouched.
    let Some(position) = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.position)
    else {
        return;
    };
    match world.resource::<Map>().movement_model() {
        // A continuous mover's position is a free point with no crossing
        // state to finish — watchers may always interrupt.
        MovementModel::Continuous => {}
        MovementModel::Cell => {
            if movement_model::is_mid_crossing(position) {
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
    let canceled = dispatch_cancel(
        entity,
        &front_order,
        CancelPolicy::Force,
        front_state,
        world,
    );
    debug_assert_eq!(
        canceled.state,
        OrderState::Finished,
        "a force-canceled sub-order must stop immediately"
    );
    queue.0.pop_front();
    queue.push_front(replacement);

    let operation = entity_def::operation(world, entity);
    prepare_front(entity, operation, queue, world);
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
/// - A `Cancels` follow-up: the rest of the queue is canceled with its policy
///   and the cancels applied at once.
///
/// A front order whose conduct holds is skipped without dispatch; it resumes
/// untouched once the entity operates again and stands on the map. One whose
/// conduct cancels is skipped too, and the next [`prepare_tick`] flushes it.
///
/// After this call the front entry (if any) is `New`, `InProcessing`, or `Suspended`.
pub fn process_tick(entity: Entity, queue: &mut OrderQueueComponent, world: &mut World) {
    let Some(front) = queue.0.front() else { return };
    let operation = entity_def::operation(world, entity);
    match conduct(world, entity, operation, &front.order, front.state) {
        Conduct::Holds | Conduct::Cancels => return,
        Conduct::Completes => {}
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
            FollowUp::Dies | FollowUp::Cancels(_) => result.state == OrderState::Finished,
            FollowUp::None => true,
        },
        "a sub-order goes with Suspended, a death or a cancel with Finished"
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
        FollowUp::SubOrder(sub_order) => {
            // A sub-order of an entry marked for cancel — marked by something
            // outside this queue earlier in the tick — goes with it.
            let mark = queue.front().and_then(|parent| parent.cancel);
            queue.push_front(sub_order);
            queue
                .front_mut()
                .expect("the sub-order was just pushed")
                .cancel = mark;
        }
        FollowUp::Dies => queue.push(Order::Die, Some(CancelPolicy::Force)),
        FollowUp::Cancels(policy) => {
            queue.cancel_all(policy);
            flush_canceled(entity, queue, world);
        }
    }

    debug_assert!(
        queue.0.front().is_none_or(|e| matches!(
            e.state,
            OrderState::New | OrderState::InProcessing | OrderState::Suspended
        )),
        "after process, front must be New, InProcessing, Suspended, or queue must be empty"
    );
}

/// Fits every alive entity's [`IdlenessComponent`] to its queue: one that is
/// not [`entity_def::idle`] is busy, and one that is is idle since the tick it
/// was first found so.
pub fn note_idleness(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        let idle = entity_def::idle(world, entity);
        let was = world.entity(entity).get::<IdlenessComponent>().copied();
        let now = match (idle, was) {
            (false, _) => IdlenessComponent::Busy,
            (true, Some(IdlenessComponent::Busy) | None) => IdlenessComponent::IdleSince(tick),
            (true, Some(was)) => was,
        };
        if was != Some(now) {
            world.entity_mut(entity).insert(now);
        }
    }
}

/// Prepare the front entry until it is `InProcessing` or held by its conduct,
/// or the queue is empty.
pub(crate) fn prepare_front(
    entity: Entity,
    operation: Operation,
    queue: &mut OrderQueueComponent,
    world: &mut World,
) {
    while let Some(front) = queue.front() {
        let state = front.state;
        let order = front.order.clone();

        debug_assert!(
            !matches!(state, OrderState::Finished),
            "Finished entries must never stay in the queue"
        );

        // A held front is left as it is, not started.
        match conduct(world, entity, operation, &order, state) {
            Conduct::Holds => break,
            Conduct::Completes | Conduct::Cancels => {}
        }

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
        queue.front().is_none_or(|e| {
            let held = conduct(world, entity, operation, &e.order, e.state);
            e.state == OrderState::InProcessing || matches!(held, Conduct::Holds)
        }),
        "after prepare, the front is InProcessing or held, or the queue is empty"
    );
}
