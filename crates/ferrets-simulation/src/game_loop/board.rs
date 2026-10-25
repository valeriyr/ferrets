//! Board order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    chase::{self, Destination},
    gateways::{self, Entry, Intake, Through},
    orders::{self, Processing, Refusal},
    transporters,
};
use crate::{
    berths,
    components::{
        order_queue::{CancelPolicy, OrderState},
        transport::BoardComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    session::GameSession,
};
use ferrets_content::{entity_stats::EntityStatId, holder::Holder};

/// Whether `entity` may start this Board: its type rides and it operates, the
/// holder is there and operating, and it takes this entity in (see
/// `would_board`).
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let target_id = order
        .board_target()
        .expect("Board order must have a target");
    if !rides(world, entity) {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return Err(Refusal::TargetGone);
    };
    orders::target_operating(world, target)?;
    would_board(world, entity, target)
}

/// Called once when a Board order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target_id = order
        .board_target()
        .expect("Board order must have a target");

    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    world
        .entity_mut(entity)
        .insert(BoardComponent::new(target_id));
    OrderState::InProcessing
}

/// Called when a Board order resumes from `Suspended` (its walk to the holder
/// just finished). The driver component survives suspension; validation happens in
/// [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Board entry that has a cancel policy.
///
/// Boarding stops immediately under both policies: nothing changed hands until the
/// walk arrived, and the transfer itself is atomic within a tick.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> Processing {
    world.entity_mut(entity).remove::<BoardComponent>();
    Processing::state(OrderState::Finished)
}

/// Whether a Board can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance a Board order by one tick.
///
/// Walk to within the holder's `load_range` (suspending on a chase move),
/// then go in: aboard a transporter, joining its passengers, or through a
/// gateway, into its link or across its tunnel, a crossing body keeping the
/// orders queued behind this one. Everything is re-checked every tick — a
/// holder that filled up, changed hands or stopped operating on the way turns
/// the boarder away, and one still on its boarding cooldown keeps it waiting
/// in place.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut board) = world.entity_mut(entity).take::<BoardComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let target_id = board.target;

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return Processing::state(OrderState::Finished);
    };
    if orders::target_operating(world, target).is_err()
        || would_board(world, entity, target).is_err()
    {
        return Processing::state(OrderState::Finished);
    }

    match chase::advance_to_entity(
        &mut board.last_chase,
        world,
        entity,
        target,
        entity_def::effective_stat_u32(world, target, EntityStatId::LOAD_RANGE),
    ) {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            world.entity_mut(entity).insert(board);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }

    // The holder meters admissions: a boarder arriving inside the cooldown holds
    // its place in the open until its turn comes.
    let tick = world.resource::<GameSession>().tick();
    if tick < boarding_ready_at(world, target) {
        world.entity_mut(entity).insert(board);
        return Processing::state(OrderState::InProcessing);
    }

    match &entity_def::of(world, target).holder {
        Some(Holder::Transporter(_)) => {
            transporters::admit(world, target, entity);
            // Aboard, the passenger runs nothing of its own: what it had
            // queued behind the boarding goes as a Load drops it, production
            // surviving to resume on the far side of the ride.
            Processing::finished_canceling(CancelPolicy::Soft)
        }
        Some(Holder::Gateway(_)) => match gateways::take_in(world, target, entity) {
            // Held, it runs nothing of its own, as a passenger aboard.
            Intake::Taken(Through::Into(_)) => Processing::finished_canceling(CancelPolicy::Soft),
            // Across, it carries on with what it queued behind the crossing,
            // from where it came out.
            Intake::Taken(Through::Across { .. }) => Processing::state(OrderState::Finished),
            // Nothing in the other end's unload range takes it yet: it waits
            // its turn again.
            Intake::Waits => {
                world.entity_mut(entity).insert(board);
                Processing::state(OrderState::InProcessing)
            }
        },
        None => unreachable!("the boarding was judged this tick"),
    }
}

/// Whether `target` takes `entity` in: this entity rides (else
/// [`Refusal::Incapable`]), is not changing form nor sitting in a berth, and
/// seats nobody in its own unless it crosses a tunnel (else
/// [`Refusal::Busy`]), and the target is another entity, a transporter that
/// takes it aboard or a gateway it may enter (else [`Refusal::TargetUnfit`]).
pub(crate) fn would_board(world: &World, entity: Entity, target: Entity) -> Result<(), Refusal> {
    if !rides(world, entity) {
        return Err(Refusal::Incapable);
    }
    if entity == target {
        return Err(Refusal::TargetUnfit);
    }
    if entity_def::changing(world, entity) || entity_def::attached(world, entity) {
        return Err(Refusal::Busy);
    }
    // A body seating others goes only where it stays on the map: across a
    // tunnel, its workers carried with it.
    let seats_others = berths::seated_by_others(world, entity, &[]);
    match &entity_def::of(world, target).holder {
        Some(Holder::Transporter(_)) => {
            transporters::would_take(world, entity, target)?;
            if seats_others {
                return Err(Refusal::Busy);
            }
            Ok(())
        }
        Some(Holder::Gateway(_)) => match gateways::would_enter(world, entity, target)? {
            Entry::Held(_) if seats_others => Err(Refusal::Busy),
            Entry::Held(_) | Entry::Crosses(_) => Ok(()),
        },
        None => Err(Refusal::TargetUnfit),
    }
}

/// The earliest tick `holder` may take its next body in at.
pub(super) fn boarding_ready_at(world: &World, holder: Entity) -> u32 {
    match &entity_def::of(world, holder).holder {
        Some(Holder::Transporter(_)) => transporters::boarding_ready_at(world, holder),
        Some(Holder::Gateway(_)) => gateways::boarding_ready_at(world, holder),
        None => unreachable!("only a holder paces boarding"),
    }
}

/// Whether `entity`'s type rides: it carries the cargo_size stat and moves.
fn rides(world: &World, entity: Entity) -> bool {
    let def = entity_def::of(world, entity);
    def.base_stat(EntityStatId::CARGO_SIZE).is_some() && def.can_move()
}
