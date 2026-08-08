//! Harvest order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

use super::{
    chase::{self, Destination},
    crew,
    orders::Processing,
    work,
};
use crate::{
    components::{
        build::UnderConstructionComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        resource::{
            HarvestComponent, HarvestingComponent, ResourceCarrierComponent,
            ResourceSourceComponent, UnderHarvestComponent,
        },
    },
    content::{
        entity_stats::EntityStatId,
        resource::{DepletionPolicy, ResourceCarrierDef},
        work::WorkPresence,
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::Order,
    resources::PlayerResources,
    session::player_slot::PlayerId,
    simulation_id::SimulationId,
    spawn,
};

/// How close the carrier must be to a storage to hand its load over, in grid cells.
///
/// Fixed rather than the carrier's `harvest_range`: how far a worker can reach into
/// a seam says nothing about how close it has to get to put the load down.
const DELIVERY_DISTANCE: u32 = 1;

/// How far away a replacement source may be when the current one is gone, in
/// grid cells.
const SOURCE_SEARCH_RADIUS: u32 = 12;

/// Called once when a Harvest order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately if the entity cannot carry resources or the target is neither a
/// source nor a storage.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");
    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return OrderState::Finished;
    };
    let Some(carrier_def) = entity_def::of(world, entity).resource_carrier.as_ref() else {
        return OrderState::Finished;
    };

    let target_ref = world.entity(target);
    let target_def = entity_def::of(world, target);
    let is_source = target_ref.contains::<ResourceSourceComponent>()
        && target_def
            .resource_source
            .as_ref()
            .is_some_and(|source| carrier_def.can_carry(source.kind()));
    let is_storage = target_def.resource_storage.is_some();
    if !is_source && !is_storage {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(HarvestComponent {
        source: is_source.then_some(target_id),
        ..Default::default()
    });
    OrderState::InProcessing
}

/// Called when a Harvest order resumes from `Suspended` (its walk just finished).
/// The driver component survives suspension; validation happens in [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Harvest entry that has a cancel policy.
///
/// Harvesting stops immediately under both policies: the trip in progress gives its
/// source back up and the carrier stops working it. Carried resources stay with the
/// carrier.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    if let Some(mut harvest_component) = world.entity_mut(entity).take::<HarvestComponent>() {
        end_trip_or_retry(world, entity, &mut harvest_component);
    }
    OrderState::Finished
}

/// Advance a Harvest order by one tick.
///
/// Each tick the carrier either delivers or harvests:
///
/// - **Deliver** when carrying a full load, when the order targeted a storage and
///   the initial load has not been dropped off yet, or when no source is left.
///   Walks to the nearest accepting storage of the owner and adds the load to the
///   player's stockpile.
/// - **Harvest** otherwise: walks to the source, takes it up as the carrier's
///   declared presence for the kind allows — waiting in place while a source it
///   cannot share is worked — and works for the source's harvest time, then
///   transfers up to a full load. A depleted source is destroyed or left empty
///   on the map, per its [`DepletionPolicy`].
///
/// The loop continues until no source is available and nothing is carried, the
/// carried load cannot be delivered anywhere, or the carrier cannot reach its
/// destination.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let Some(mut harvest_component) = world.entity_mut(entity).take::<HarvestComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let result = advance(entity, order, &mut harvest_component, world);

    // Whichever way the order ends, it ends here — one place stops the work and gives
    // the source back up, so no way out of [`advance`] has to remember to.
    match result.state {
        OrderState::Finished => end_trip_or_retry(world, entity, &mut harvest_component),
        OrderState::InProcessing | OrderState::Suspended => {
            world.entity_mut(entity).insert(harvest_component);
        }
        OrderState::New => unreachable!("advance never returns an order to New"),
    }

    result
}

/// One tick of the deliver-or-harvest loop, with the driver component held out of the
/// world for the duration: the caller puts it back or drops it, per the state returned.
fn advance(
    entity: Entity,
    order: &Order,
    harvest_component: &mut HarvestComponent,
    world: &mut World,
) -> Processing {
    let carrier_def = entity_def::of(world, entity)
        .resource_carrier
        .as_ref()
        .unwrap()
        .clone();

    // A trip whose source vanished mid-work is abandoned: the carrier stops working
    // and comes back onto the map before anything else happens.
    if let Some(harvesting_id) = harvest_component.harvesting
        && world
            .resource::<EntityIndex>()
            .interactable(world, harvesting_id)
            .is_none()
        && !end_trip(world, entity, harvest_component)
    {
        return Processing::state(OrderState::InProcessing);
    }

    let (carried_kind, carried_amount) = {
        let carrier = world
            .entity(entity)
            .get::<ResourceCarrierComponent>()
            .unwrap();
        (carrier.kind.clone(), carrier.amount)
    };
    let position = world
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .position;

    let source = resolve_source(
        entity,
        order,
        harvest_component,
        carried_kind.as_deref(),
        &carrier_def,
        world,
    );

    let carrying = carried_amount > 0;
    let carried_capacity = carried_kind
        .as_deref()
        .and_then(|kind| carrier_def.harvest_data(kind))
        .map_or(0, |data| data.capacity());

    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");

    let ordered_storage_pending = !harvest_component.delivered_initial_load
        && world
            .resource::<EntityIndex>()
            .interactable(world, target_id)
            .is_some_and(|t| entity_def::of(world, t).resource_storage.is_some());
    let deliver = carrying
        && (carried_amount >= carried_capacity || ordered_storage_pending || source.is_none());

    if deliver {
        let kind = carried_kind.expect("carrying implies a resource kind");

        // A delivery interrupting a partial trip ends it: the carrier cannot walk a
        // load anywhere while it is still at work in a seam.
        if !end_trip(world, entity, harvest_component) {
            return Processing::state(OrderState::InProcessing);
        }

        let Some(player) = world
            .entity(entity)
            .get::<OwnerComponent>()
            .map(|o| o.player())
        else {
            return Processing::state(OrderState::Finished);
        };
        let Some(storage) = resolve_storage(entity, order, &kind, player, world) else {
            return Processing::state(OrderState::Finished);
        };

        match chase::advance_to_entity(
            &mut harvest_component.last_chase,
            world,
            position,
            storage,
            DELIVERY_DISTANCE,
        ) {
            Destination::OutOfReach => return Processing::state(OrderState::Finished),
            Destination::Walk(move_order) => return Processing::suspend(move_order),
            Destination::Arrived => {}
        }

        chase::face_entity(world, entity, storage);
        world
            .resource_mut::<PlayerResources>()
            .add(player, &kind, carried_amount);
        let mut entity_mut = world.entity_mut(entity);
        let mut carrier = entity_mut.get_mut::<ResourceCarrierComponent>().unwrap();
        carrier.kind = None;
        carrier.amount = 0;
        harvest_component.delivered_initial_load = true;

        return Processing::state(OrderState::InProcessing);
    }

    let Some(source_id) = source else {
        // Nothing carried and no source left.
        return Processing::state(OrderState::Finished);
    };
    let source_entity = world
        .resource::<EntityIndex>()
        .alive(source_id)
        .expect("resolve_source returns alive sources");
    harvest_component.source = Some(source_id);

    match chase::advance_to_entity(
        &mut harvest_component.last_chase,
        world,
        position,
        source_entity,
        work::reach(world, entity, EntityStatId::HARVEST_RANGE),
    ) {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => return Processing::suspend(move_order),
        Destination::Arrived => {}
    }

    chase::face_entity(world, entity, source_entity);

    let (source_kind, depletion) = {
        let source_def = entity_def::of(world, source_entity)
            .resource_source
            .as_ref()
            .unwrap();
        (source_def.kind().to_string(), source_def.depletion())
    };
    let harvest_data = *carrier_def
        .harvest_data(&source_kind)
        .expect("resolve_source returns carryable kinds");

    // Take up the source, waiting in place while one that cannot be shared is
    // worked by somebody else.
    if harvest_component.harvesting != Some(source_id) {
        if source_excludes(world, source_entity, entity, &source_kind) {
            return Processing::state(OrderState::InProcessing);
        }
        begin_trip(
            world,
            entity,
            source_entity,
            harvest_component,
            harvest_data.presence(),
        );
    }

    harvest_component.progress += 1;

    if harvest_component.progress >= harvest_data.harvest_time() {
        // The carrier must be back on the map before the load can move on.
        if !end_trip(world, entity, harvest_component) {
            return Processing::state(OrderState::InProcessing);
        }

        let available = world
            .entity(source_entity)
            .get::<ResourceSourceComponent>()
            .unwrap()
            .amount;
        let take = harvest_data
            .capacity()
            .saturating_sub(carried_amount)
            .min(available);

        {
            let mut entity_mut = world.entity_mut(entity);
            let mut carrier = entity_mut.get_mut::<ResourceCarrierComponent>().unwrap();
            carrier.kind = Some(source_kind);
            carrier.amount += take;
        }

        let remaining = {
            let mut source_mut = world.entity_mut(source_entity);
            let mut source_resources = source_mut.get_mut::<ResourceSourceComponent>().unwrap();
            source_resources.amount -= take;
            source_resources.amount
        };

        if remaining == 0 {
            match depletion {
                DepletionPolicy::Destroy => spawn::destroy_entity(world, source_entity),
                DepletionPolicy::Persist => {}
            }
        }
    }

    Processing::state(OrderState::InProcessing)
}

/// Whether `entity` is shut out of working `source` for `kind` by the crew already
/// on it.
fn source_excludes(world: &World, source: Entity, entity: Entity, kind: &str) -> bool {
    crew::excludes::<UnderHarvestComponent>(world, source, entity, |world, carrier| {
        shares_sources(world, carrier, kind)
    })
}

/// Whether an entity's carrying capability lets several workers share one source
/// of `kind`.
fn shares_sources(world: &World, entity: Entity, kind: &str) -> bool {
    entity_def::of(world, entity)
        .resource_carrier
        .as_ref()
        .and_then(|carrier| carrier.harvest_data(kind))
        .is_some_and(|data| data.presence().stacks())
}

/// Starts a trip on `source`: the carrier takes the source up, joining its crew, and
/// is at work from now until [`end_trip`].
///
/// Being at work and being off the map are separate facts — a carrier inside a seam is
/// both — so the mark goes on regardless, and the presence decides only whether the
/// carrier leaves the map for the duration.
fn begin_trip(
    world: &mut World,
    entity: Entity,
    source: Entity,
    harvest: &mut HarvestComponent,
    presence: WorkPresence,
) {
    harvest.harvesting = Some(entity_def::simulation_id(world, source));
    harvest.progress = 0;
    crew::join::<UnderHarvestComponent>(world, source, entity);

    world.entity_mut(entity).insert(HarvestingComponent);
    work::enter(world, entity, presence);
}

/// Ends the trip in progress: the carrier stops working, comes back onto the map if it
/// was inside the source, and gives the source back up.
///
/// Returns `false` when a hidden carrier has no free cell to reappear on. The trip
/// stands in that case — it is still at work and still holds its source — and the
/// caller retries next tick.
fn end_trip(world: &mut World, entity: Entity, harvest: &mut HarvestComponent) -> bool {
    let (anchor, size) = own_footprint(world, entity);
    if !spawn::reveal_entity_near(world, entity, anchor, size) {
        return false;
    }

    stop_work(world, entity, harvest);
    true
}

/// Like [`end_trip`], but for the paths that cannot retry: the trip ends either way,
/// and a carrier with nowhere to reappear stays hidden with the reveal queued for a
/// later tick.
fn end_trip_or_retry(world: &mut World, entity: Entity, harvest: &mut HarvestComponent) {
    stop_work(world, entity, harvest);

    let (anchor, size) = own_footprint(world, entity);
    work::leave(world, entity, anchor, size);
}

/// Puts down the work: the carrier is at it no longer, and its source is given back up
/// — with [`UnderHarvestComponent`] going as the last carrier out of the crew.
///
/// A source already on its way off the map keeps nothing to leave.
fn stop_work(world: &mut World, entity: Entity, harvest: &mut HarvestComponent) {
    world.entity_mut(entity).remove::<HarvestingComponent>();

    let Some(source_id) = harvest.harvesting.take() else {
        return;
    };
    harvest.progress = 0;

    if let Some(source) = world.resource::<EntityIndex>().alive(source_id) {
        crew::leave_and_unmark::<UnderHarvestComponent>(world, source, entity);
    }
}

/// The cell and footprint size the entity stands on, used as the reveal anchor.
fn own_footprint(world: &World, entity: Entity) -> (CellPos, CellSize) {
    let (position, size) = entity_def::footprint(world, entity);
    (CellPos::from(position), size)
}

/// Picks the source to harvest from: the trip in progress, the ordered target,
/// the last source worked, or the nearest matching source within
/// [`SOURCE_SEARCH_RADIUS`] — in that priority order.
///
/// Only sources of kinds the carrier can carry qualify; when the carrier holds
/// a partial load, only sources of the same resource kind do.
fn resolve_source(
    entity: Entity,
    order: &Order,
    hc: &HarvestComponent,
    carried_kind: Option<&str>,
    carrier_def: &ResourceCarrierDef,
    world: &World,
) -> Option<SimulationId> {
    let matches = |id: SimulationId| -> bool {
        let Some(source) = world.resource::<EntityIndex>().interactable(world, id) else {
            return false;
        };
        let source_ref = world.entity(source);
        let Some(source_def) = entity_def::of(world, source).resource_source.as_ref() else {
            return false;
        };
        source_ref
            .get::<ResourceSourceComponent>()
            .is_some_and(|s| s.amount > 0)
            && carrier_def.can_carry(source_def.kind())
            && carried_kind.is_none_or(|kind| source_def.kind() == kind)
    };

    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");

    for candidate in [hc.harvesting, Some(target_id), hc.source]
        .into_iter()
        .flatten()
    {
        if matches(candidate) {
            return Some(candidate);
        }
    }

    let position = CellPos::from(
        world
            .entity(entity)
            .get::<LocationComponent>()
            .unwrap()
            .position,
    );
    nearest(world, position, Some(SOURCE_SEARCH_RADIUS), |id, _| {
        matches(id)
    })
}

/// Picks the storage to deliver to: the ordered target if it qualifies, otherwise
/// the nearest finished storage of the owner that accepts `kind`.
fn resolve_storage(
    entity: Entity,
    order: &Order,
    kind: &str,
    player: PlayerId,
    world: &World,
) -> Option<Entity> {
    let qualifies = |storage: Entity| -> bool {
        let storage_ref = world.entity(storage);
        entity_def::of(world, storage)
            .resource_storage
            .as_ref()
            .is_some_and(|s| s.accepts(kind))
            && storage_ref
                .get::<OwnerComponent>()
                .is_some_and(|o| o.player() == player)
            && !storage_ref.contains::<UnderConstructionComponent>()
    };

    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");

    if let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
        && qualifies(target)
    {
        return Some(target);
    }

    let position = CellPos::from(
        world
            .entity(entity)
            .get::<LocationComponent>()
            .unwrap()
            .position,
    );
    let id = nearest(world, position, None, |_, e| qualifies(e))?;
    world.resource::<EntityIndex>().alive(id)
}

/// Finds the alive entity matching `filter` nearest to `from`, measured to the
/// entity's footprint with the map's projection metric. Ties break on the lower
/// [`SimulationId`], so the result is deterministic.
fn nearest(
    world: &World,
    from: CellPos,
    max_distance: Option<u32>,
    filter: impl Fn(SimulationId, Entity) -> bool,
) -> Option<SimulationId> {
    let projection = world.resource::<Map>().projection();
    let mut best: Option<(u32, SimulationId)> = None;

    for (id, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !filter(id, entity) {
            continue;
        }
        let (position, size) = entity_def::footprint(world, entity);
        let origin = CellPos::from(position);
        if let Some(max) = max_distance
            && !projection.in_range_of_rect(from, CellRect::new(origin, size), max)
        {
            continue;
        }
        let distance = projection.rect_distance(from, CellRect::new(origin, size));
        // Ascending id iteration: strictly-closer wins, ties keep the lower id.
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, id));
        }
    }

    best.map(|(_, id)| id)
}
