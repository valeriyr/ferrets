//! Board order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    chase::{self, Destination},
    orders::{self, Processing, Refusal},
};
use crate::{
    berths,
    components::{
        entity_stats::StatsComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        owner,
        transport::{BoardComponent, TransporterComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    session::GameSession,
    spawn::{self, Boarding},
};
use ferrets_content::entity_stats::EntityStatId;

/// Whether `entity` may start this Board: its type rides and it operates, the
/// transporter is there and operating, and it takes this entity aboard (see
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

/// Called when a Board order resumes from `Suspended` (its walk to the transporter
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
/// Walk to within the transporter's `load_range` (suspending on a chase move),
/// then step aboard: the passenger disappears from the map and joins the
/// transporter's passengers. Everything is re-checked every tick — a transporter that
/// filled up, changed hands or stopped operating on the way turns the boarder away,
/// and one still on its boarding cooldown keeps it waiting in place.
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
    let ready_at = boarding_ready_at(world, target);
    if tick < ready_at {
        world.entity_mut(entity).insert(board);
        return Processing::state(OrderState::InProcessing);
    }

    admit(world, target, entity);
    // Aboard, the passenger runs nothing of its own: what it had queued behind
    // the boarding goes as a Load drops it, production surviving to resume on
    // the far side of the ride.
    Processing::finished_canceling(CancelPolicy::Soft)
}

/// Takes `passenger` aboard `holder`: whatever it was doing is soft-canceled
/// first, then it turns to the door, boards, and starts
/// the holder's boarding cooldown.
pub(super) fn admit(world: &mut World, holder: Entity, passenger: Entity) {
    // A passenger admitted off a job leaves the job before it leaves the map:
    // the job's cancel path sets it down, and the boarding takes it from
    // there rather than the cancel setting a boarded body back on the grid.
    // A passenger boarding on its own initiative has its queue out of the
    // world here; the loop cancels what it queued behind the boarding.
    if world.entity(passenger).contains::<OrderQueueComponent>() {
        orders::drop_all(world, passenger, CancelPolicy::Soft);
        let passenger_id = entity_def::simulation_id(world, passenger);
        assert!(
            world
                .resource::<EntityIndex>()
                .alive(passenger_id)
                .is_some(),
            "a soft cancel kills nothing on a body judged not to be changing form"
        );
    }
    chase::face_entity(world, passenger, holder);
    match spawn::board(world, passenger, holder) {
        Boarding::Boarded => {}
        Boarding::Refused(refusal) => {
            unreachable!("an admission was judged this tick, got {refusal:?}")
        }
    }

    let tick = world.resource::<GameSession>().tick();
    let period = entity_def::effective_ticks(world, holder, EntityStatId::LOAD_PERIOD);
    world
        .entity_mut(holder)
        .get_mut::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
        .boarding_ready_at = tick + period;
}

/// Whether `entity`'s type rides: it carries the cargo_size stat and moves,
/// since boarding is a walk.
fn rides(world: &World, entity: Entity) -> bool {
    let def = entity_def::of(world, entity);
    def.base_stat(EntityStatId::CARGO_SIZE).is_some() && def.can_move()
}

/// Whether `target` takes `entity` aboard: this entity rides (else
/// [`Refusal::Incapable`]), is not changing form, sitting in a berth or seating
/// anyone in its own (else [`Refusal::Busy`]), and the target is another
/// entity that takes passengers its policy and admission list accept with
/// room aboard for this one (else [`Refusal::TargetUnfit`]).
pub(crate) fn would_board(world: &World, entity: Entity, target: Entity) -> Result<(), Refusal> {
    if !rides(world, entity) {
        return Err(Refusal::Incapable);
    }
    if entity == target {
        return Err(Refusal::TargetUnfit);
    }
    if entity_def::changing(world, entity)
        || entity_def::attached(world, entity)
        || berths::seated_by_others(world, entity, &[])
    {
        return Err(Refusal::Busy);
    }
    let entity_def = entity_def::of(world, entity);
    let target_def = entity_def::of(world, target);
    let Some(transporter) = target_def.transporter.as_ref() else {
        return Err(Refusal::TargetUnfit);
    };

    // Read from the holder: whom it admits is its own rule about its riders.
    let allowed = owner::admits(
        world.resource::<GameSession>(),
        transporter.boarding(),
        entity_def::owner(world, target),
        entity_def::owner(world, entity),
    );
    if !allowed {
        return Err(Refusal::TargetUnfit);
    }

    if !transporter.carries().admits(entity_def) {
        return Err(Refusal::TargetUnfit);
    }

    // The capacity is a stat, so a modifier can enlarge or shrink the hold; a
    // hold shrunk below its occupancy keeps everyone aboard and admits nobody.
    if occupied_slots(world, target) + cargo_size(world, entity)
        > entity_def::effective_stat_u32(world, target, EntityStatId::CARGO_CAPACITY)
    {
        return Err(Refusal::TargetUnfit);
    }
    Ok(())
}

/// The earliest tick `holder` may take its next passenger at.
pub(super) fn boarding_ready_at(world: &World, holder: Entity) -> u32 {
    world
        .entity(holder)
        .get::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
        .boarding_ready_at
}

/// The slots taken by everyone already aboard `holder`.
pub(super) fn occupied_slots(world: &World, holder: Entity) -> u32 {
    let Some(transporter) = world.entity(holder).get::<TransporterComponent>() else {
        return 0;
    };
    transporter
        .passengers
        .iter()
        .filter_map(|&id| world.resource::<EntityIndex>().alive(id))
        .map(|passenger| cargo_size(world, passenger))
        .sum()
}

/// The slots `passenger` takes aboard a transporter.
fn cargo_size(world: &World, passenger: Entity) -> u32 {
    world
        .entity(passenger)
        .get::<StatsComponent>()
        .expect("a transportable entity has a stat store")
        .effective_as_u32(EntityStatId::CARGO_SIZE)
        .expect("a transportable entity carries cargo_size")
}
