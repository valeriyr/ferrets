//! What a transporter does with a body that boards it: takes it aboard, paced
//! by its boarding period, and sets it down again beside itself.

use bevy_ecs::{
    entity::Entity,
    world::{Mut, World},
};
use ferrets_content::{entity_stats::EntityStatId, transport::TransporterDef};

use super::{
    chase,
    orders::{self, Refusal},
};
use crate::{
    beside::Lean,
    components::{
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner,
        transport::{BoardedComponent, GarrisonFireComponent, TransporterComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    session::GameSession,
    simulation_id::SimulationId,
    spawn::{self, Boarding, SetDown},
};

/// Whether `transporter` takes `entity` aboard now: its boarding admits
/// `entity`'s owner, its `carries` admits `entity`'s type, and there is room
/// aboard for it (else [`Refusal::TargetUnfit`]).
pub(crate) fn would_take(
    world: &World,
    entity: Entity,
    transporter: Entity,
) -> Result<(), Refusal> {
    let def = transporter_def(world, transporter);

    // Read from the holder: whom it admits is its own rule about its riders.
    let allowed = owner::admits(
        world.resource::<GameSession>(),
        def.boarding(),
        entity_def::owner(world, transporter),
        entity_def::owner(world, entity),
    );
    if !allowed || !def.carries().admits(entity_def::of(world, entity)) {
        return Err(Refusal::TargetUnfit);
    }

    // The capacity is a stat, so a modifier can enlarge or shrink the hold; a
    // hold shrunk below its occupancy keeps everyone aboard and admits nobody.
    if occupied_slots(world, transporter) + entity_def::cargo_size(world, entity)
        > entity_def::effective_stat_u32(world, transporter, EntityStatId::CARGO_CAPACITY)
    {
        return Err(Refusal::TargetUnfit);
    }
    Ok(())
}

/// Takes `passenger` aboard `transporter`: whatever it was doing is
/// soft-canceled first, then it turns to the door, boards, and starts the
/// transporter's boarding pace.
pub(crate) fn admit(world: &mut World, transporter: Entity, passenger: Entity) {
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
    chase::face_entity(world, passenger, transporter);
    match spawn::board(world, passenger, transporter) {
        Boarding::Boarded => {}
        Boarding::Refused(refusal) => {
            unreachable!("an admission was judged this tick, got {refusal:?}")
        }
    }

    let tick = world.resource::<GameSession>().tick();
    let period = entity_def::effective_ticks(world, transporter, EntityStatId::LOAD_PERIOD);
    transporter_state_mut(world, transporter).boarding_ready_at = tick + period;
}

/// The earliest tick `transporter` may take its next passenger at.
pub(crate) fn boarding_ready_at(world: &World, transporter: Entity) -> u32 {
    transporter_state(world, transporter).boarding_ready_at
}

/// The first passenger aboard `transporter` by id, past `after` when given.
pub(crate) fn next_aboard(
    world: &World,
    transporter: Entity,
    after: Option<SimulationId>,
) -> Option<SimulationId> {
    transporter_state(world, transporter)
        .passengers
        .iter()
        .copied()
        .find(|&id| after.is_none_or(|after| id > after))
}

/// Sets `passenger` down within `transporter`'s unload range, leaning as
/// `lean` says. Out, it is aboard no longer.
pub(crate) fn let_out(
    world: &mut World,
    transporter: Entity,
    passenger: Entity,
    lean: Lean,
) -> SetDown {
    let transporter_id = entity_def::simulation_id(world, transporter);
    let set_down = spawn::place_back_from_holder(world, passenger, transporter_id, lean);
    match set_down {
        SetDown::Placed => {
            let passenger_id = entity_def::simulation_id(world, passenger);
            transporter_state_mut(world, transporter)
                .passengers
                .remove(&passenger_id);
            world
                .entity_mut(passenger)
                .remove::<(BoardedComponent, GarrisonFireComponent)>();
        }
        SetDown::NoCell => {}
    }
    set_down
}

/// Takes `passenger`, gone from the world, off `transporter`'s list.
pub(crate) fn drop_stale(world: &mut World, transporter: Entity, passenger: SimulationId) {
    transporter_state_mut(world, transporter)
        .passengers
        .remove(&passenger);
}

/// The slots taken by everyone aboard `transporter`.
fn occupied_slots(world: &World, transporter: Entity) -> u32 {
    transporter_state(world, transporter)
        .passengers
        .iter()
        .filter_map(|&id| world.resource::<EntityIndex>().alive(id))
        .map(|passenger| entity_def::cargo_size(world, passenger))
        .sum()
}

/// `transporter`'s content definition.
///
/// Panics for an entity that is no transporter.
fn transporter_def(world: &World, transporter: Entity) -> &TransporterDef {
    entity_def::of(world, transporter)
        .transporter()
        .expect("only a transporter takes passengers aboard")
}

/// `transporter`'s live state.
///
/// Panics for an entity that is no transporter.
fn transporter_state(world: &World, transporter: Entity) -> &TransporterComponent {
    world
        .entity(transporter)
        .get::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
}

/// `transporter`'s live state, to change.
///
/// Panics for an entity that is no transporter.
fn transporter_state_mut(world: &mut World, transporter: Entity) -> Mut<'_, TransporterComponent> {
    world
        .entity_mut(transporter)
        .into_mut::<TransporterComponent>()
        .expect("a transporter holds a passenger list")
}
