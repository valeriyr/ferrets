//! Board order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};

use super::{
    chase::{self, Destination},
    crew,
    orders::Processing,
};
use crate::{
    components::{
        entity_stats::StatsComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        owner::OwnerComponent,
        tags::TagsComponent,
        transport::{BoardComponent, BoardedComponent, TransporterComponent},
    },
    content::{entity_stats::EntityStatId, transport::BoardingPolicy},
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    selection::Selection,
    session::GameSession,
    spawn,
};

/// Called once when a Board order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the target will not take this entity aboard — see
/// [`would_board`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target_id = order
        .board_target()
        .expect("Board order must have a target");

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return OrderState::Finished;
    };
    if !would_board(world, entity, target) {
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
) -> OrderState {
    world.entity_mut(entity).remove::<BoardComponent>();
    OrderState::Finished
}

/// Advance a Board order by one tick.
///
/// Walk to within the transporter's `load_range` (suspending on a chase move),
/// then step aboard: the passenger disappears from the map and joins the
/// transporter's crew. Everything is re-checked at arrival — a transporter that
/// filled up or changed hands on the way turns the boarder away, and one still
/// on its boarding cooldown keeps it waiting in place.
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
    if !would_board(world, entity, target) {
        return Processing::state(OrderState::Finished);
    }

    match chase::advance_to_entity(
        &mut board.last_chase,
        world,
        entity_def::position(world, entity),
        target,
        entity_def::stat_u32(world, target, EntityStatId::LOAD_RANGE),
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

    Processing::state(OrderState::Finished)
}

/// Takes `passenger` aboard `holder`: the passenger turns to the door, leaves
/// the map, joins the crew, and starts the holder's boarding cooldown.
///
/// Whatever else the passenger had queued is force-cancelled — an entity off
/// the map must not keep walking its old orders. (A passenger boarding on its
/// own initiative has its queue held out of the world during its dispatch, so
/// for it this is a no-op; its board order is the front entry anyway.)
pub(super) fn admit(world: &mut World, holder: Entity, passenger: Entity) {
    if let Some(mut queue) = world.entity_mut(passenger).get_mut::<OrderQueueComponent>() {
        queue.cancel_all(CancelPolicy::Force);
    }

    chase::face_entity(world, passenger, holder);
    spawn::hide_entity(world, passenger);
    crew::join_existing::<TransporterComponent>(world, holder, passenger);
    let holder_id = entity_def::simulation_id(world, holder);
    world
        .entity_mut(passenger)
        .insert(BoardedComponent { holder: holder_id });

    let tick = world.resource::<GameSession>().tick();
    let period = entity_def::stat_u32(world, holder, EntityStatId::LOAD_PERIOD);
    world
        .entity_mut(holder)
        .get_mut::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
        .boarding_ready_at = tick + period;

    // A unit that just left the map has no business staying in the selection.
    let id = entity_def::simulation_id(world, passenger);
    world.resource_mut::<Selection>().remove(id);
}

/// Whether sending `entity` to `target` should be read as an intent to board it:
/// the target takes passengers, this entity is a transportable unit the target's
/// policy and admission list accept, and there is room aboard for it.
pub(super) fn would_board(world: &World, entity: Entity, target: Entity) -> bool {
    let target_def = entity_def::of(world, target);
    let Some(transporter) = target_def.transporter.as_ref() else {
        return false;
    };

    let entity_def = entity_def::of(world, entity);
    // Transportability = carrying the cargo_size stat; boarding is a walk, so a
    // passenger must also be able to make it.
    if entity_def.base_stat(EntityStatId::CARGO_SIZE).is_none() || !entity_def.can_move() {
        return false;
    }

    let allowed = match (
        world.entity(entity).get::<OwnerComponent>(),
        world.entity(target).get::<OwnerComponent>(),
    ) {
        (Some(rider), Some(holder)) => match transporter.boarding() {
            BoardingPolicy::Own => rider.player() == holder.player(),
            BoardingPolicy::Allies => world
                .resource::<GameSession>()
                .are_allied(rider.player(), holder.player()),
        },
        // A passenger belongs to somebody, and a neutral holder to nobody.
        _ => false,
    };
    if !allowed {
        return false;
    }

    if !transporter.admits(
        &entity_def.name,
        world.entity(entity).get::<TagsComponent>(),
    ) {
        return false;
    }

    // The capacity is a stat, so a modifier can enlarge or shrink the hold; a
    // hold shrunk below its occupancy keeps everyone aboard and admits nobody.
    occupied_slots(world, target) + cargo_size(world, entity)
        <= entity_def::stat_u32(world, target, EntityStatId::CARGO_CAPACITY)
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
