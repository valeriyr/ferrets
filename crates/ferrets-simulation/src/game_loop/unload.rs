//! Unload order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;

use super::{
    chase::{self, Destination},
    gateways,
    orders::{self, Processing, Refusal},
    transporters,
};
use crate::{
    beside::Lean,
    components::{
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        rally::RallyTarget,
        transport::UnloadComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    order::Order,
    rally,
    simulation_id::SimulationId,
    spawn::SetDown,
    visibility,
};
use ferrets_content::{entity_stats::EntityStatId, holder::Holder};

/// Whether `entity` may start an Unload: it is a transporter, or a gateway
/// that lets bodies out of its link; it operates; and somebody is
/// aboard or inside.
pub fn can_start(world: &World, entity: Entity, _order: &Order) -> Result<(), Refusal> {
    let lets_out = match &entity_def::of(world, entity).holder {
        Some(Holder::Transporter(_)) => true,
        Some(Holder::Gateway(_)) => gateways::lets_out(world, entity),
        None => false,
    };
    if !lets_out {
        return Err(Refusal::Incapable);
    }
    orders::requires_operating(world, entity)?;
    if next_out(world, entity, None).is_none() {
        return Err(Refusal::NothingToDo);
    }
    Ok(())
}

/// Called once when an Unload order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let at = order
        .unload_at()
        .expect("Unload order carries a destination");

    if can_start(world, entity, order).is_err() {
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
) -> Processing {
    world.entity_mut(entity).remove::<UnloadComponent>();
    Processing::state(OrderState::Finished)
}

/// Whether an Unload can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance an Unload order by one tick.
///
/// With a destination, a mobile holder first walks into its `unload_range` of
/// it — an immobile one, or one whose walk can make no more progress, lets its
/// passengers out where it stands. Passengers then step out one per
/// `unload_period` ticks (a zero period empties the hold at once), each set
/// down within the holder's `unload_range` — a transporter's on the side of
/// where it is sent, a gateway's on the free cell nearest the destination — and
/// sent on: to the destination when the order named one — walking as close to
/// it as the map allows — otherwise to the holder's rally point, if set. A
/// passenger no cell it may stand on takes stays inside and the next one
/// goes; the order keeps running and retries it every tick, and finishes
/// when the hold is empty.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut unload) = world.entity_mut(entity).take::<UnloadComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    if let Some(at) = unload.at
        && entity_def::of(world, entity).can_move()
    {
        match chase::advance_to_point(
            &mut unload.last_chase,
            world,
            entity,
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

    let period = entity_def::effective_ticks(world, entity, EntityStatId::UNLOAD_PERIOD);
    // The last body this tick that nothing it may stand on took: it waits
    // inside, and the bodies after it go first.
    let mut waiting = None;
    loop {
        let Some(passenger_id) = next_out(world, entity, waiting) else {
            break;
        };

        let Some(passenger) = world.resource::<EntityIndex>().alive(passenger_id) else {
            // A passenger that died aboard left a stale entry; drop it and move on.
            drop_stale(world, entity, passenger_id);
            continue;
        };

        // A passenger steps out on the side of where it is sent: the order's
        // point, else the holder's rally point, else no side in particular.
        let lean = exit_lean(world, entity, unload.at);
        match let_out(world, entity, passenger, unload.at, lean) {
            SetDown::Placed => {}
            // No cell it may stand on: it stays inside to retry next tick,
            // and the next body tries.
            SetDown::NoCell => {
                waiting = Some(passenger_id);
                continue;
            }
        }
        dispatch(world, entity, passenger, unload.at);

        if period > 0 {
            // One tick of the spacing elapses before the next process call
            // reads the counter, so the reveal lands exactly `period` later.
            unload.cooldown = period - 1;
            break;
        }
    }

    if next_out(world, entity, None).is_none() {
        return Processing::state(OrderState::Finished);
    }
    world.entity_mut(entity).insert(unload);
    Processing::state(OrderState::InProcessing)
}

/// The first body `holder` lets out next, past `after` when given.
fn next_out(world: &World, holder: Entity, after: Option<SimulationId>) -> Option<SimulationId> {
    match &entity_def::of(world, holder).holder {
        Some(Holder::Transporter(_)) => transporters::next_aboard(world, holder, after),
        Some(Holder::Gateway(_)) => gateways::next_held(world, holder, after),
        None => unreachable!("only a holder unloads"),
    }
}

/// Takes `body`, gone from the world, off what `holder` keeps of it.
fn drop_stale(world: &mut World, holder: Entity, body: SimulationId) {
    match &entity_def::of(world, holder).holder {
        Some(Holder::Transporter(_)) => transporters::drop_stale(world, holder, body),
        Some(Holder::Gateway(_)) => gateways::drop_stale(world, holder, body),
        None => unreachable!("only a holder unloads"),
    }
}

/// Sets `body` down out of `holder`, toward `at` where the holder sets down
/// near a point, leaning as `lean` says.
fn let_out(
    world: &mut World,
    holder: Entity,
    body: Entity,
    at: Option<FixedUVec2>,
    lean: Lean,
) -> SetDown {
    match &entity_def::of(world, holder).holder {
        Some(Holder::Transporter(_)) => transporters::let_out(world, holder, body, lean),
        Some(Holder::Gateway(_)) => gateways::let_out(world, holder, body, at, lean),
        None => unreachable!("only a holder unloads"),
    }
}

/// Sends a freshly unloaded passenger on its way: to the order's destination if
/// it named one, otherwise wherever the holder's rally point says.
fn dispatch(world: &mut World, holder: Entity, passenger: Entity, at: Option<FixedUVec2>) {
    match at {
        // A holder sends on only its own owner's units.
        Some(_) if !entity_def::same_owner(world, holder, passenger) => {}
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
        None => rally::mark_rally_due(world, holder, passenger),
    }
}

/// Which side of `holder` a passenger it lets out steps onto: toward `at`,
/// else toward the holder's rally point — an entity there only while the
/// holder's owner makes it out — else none in particular.
fn exit_lean(world: &World, holder: Entity, at: Option<FixedUVec2>) -> Lean {
    let toward_cell =
        |position: FixedUVec2| Lean::Toward(CellRect::new(CellPos::from(position), CellSize::ONE));
    match at {
        Some(at) => toward_cell(at),
        None => match rally::target_of(world, holder) {
            Some(RallyTarget::Position(position)) => toward_cell(position),
            Some(RallyTarget::Entity(id)) => {
                let target = match entity_def::owner(world, holder) {
                    Some(owner) => visibility::interactable_to(world, owner, id),
                    None => world.resource::<EntityIndex>().interactable(world, id),
                };
                match target {
                    Some(target) => Lean::Toward(entity_def::footprint_rect(world, target)),
                    None => Lean::Nothing,
                }
            }
            None => Lean::Nothing,
        },
    }
}
