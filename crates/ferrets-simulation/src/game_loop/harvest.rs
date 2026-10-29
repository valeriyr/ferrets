//! The Harvest order: a carrier's loop of working a source of one resource
//! kind and delivering what it carries to its owner's storage.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_stats::EntityStatId,
    resource::{Banking, DepletionPolicy, ResourceCarrierDef},
    work::WorkPresence,
};
use ferrets_geometry::cell_rect::CellRect;

use super::{
    chase::{self, Destination},
    crew,
    orders::{self, Processing, Refusal},
    work,
};
use crate::{
    berths,
    beside::Beside,
    components::{
        order_queue::{CancelPolicy, OrderState},
        resource::{
            HarvestComponent, HarvestingComponent, ResourceCarrierComponent,
            ResourceSourceComponent, UnderHarvestComponent,
        },
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    events::DeathCause,
    landing::ComesFrom,
    map::Map,
    order::Order,
    reach::{self, Approach},
    resources,
    session::player_id::PlayerId,
    simulation_id::SimulationId,
    spawn,
};

/// How close the carrier must be to a storage to hand its load over, in grid cells.
///
/// Fixed rather than the carrier's `harvest_range`: how far a worker can reach into
/// a seam says nothing about how close it has to get to put the load down.
const DELIVERY_DISTANCE: u32 = 1;

/// How far away a replacement source may be when the current one is gone or
/// cannot be reached, in grid cells.
const SOURCE_SEARCH_RADIUS: u32 = 12;

/// How long a carrier stands before retrying a walk it could not finish, to a
/// source or to a storage, in ticks.
const BLOCKED_WALK_RETRY_PERIOD: u32 = 8;

/// What a Harvest order reads its target as.
struct Reading {
    /// The resource kind the order works.
    kind: String,
    /// The source harvested, or `None` when the target is a storage.
    source: Option<SimulationId>,
}

/// Whether `entity` may start this Harvest: its type carries resources and it
/// operates, and the target is an operating source of a kind it carries, whose
/// work would not take it off the map from under others seated on it, or an
/// operating storage of its own that accepts the load it holds.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    reading(world, entity, order).map(|_| ())
}

/// Called once when a Harvest order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the order cannot start — see [`can_start`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Ok(Reading { kind, source }) = reading(world, entity, order) else {
        return OrderState::Finished;
    };
    world
        .entity_mut(entity)
        .insert(HarvestComponent::new(kind, source));
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
) -> Processing {
    if let Some(mut harvest_component) = world.entity_mut(entity).take::<HarvestComponent>() {
        end_trip(world, entity, &mut harvest_component);
    }
    Processing::state(OrderState::Finished)
}

/// Whether a Harvest can stand through a soft cancel: never — it drops like any
/// order a player's next command replaces.
pub fn survives_soft_cancel() -> bool {
    false
}

/// Advance a Harvest order by one tick.
///
/// Each tick the carrier either delivers or harvests:
///
/// - **Deliver** when carrying a full load of the order's kind, when the order
///   targeted a storage and the initial load has not been dropped off yet, or
///   when no source is left. Walks to the nearest accepting storage of the
///   owner and adds the load to the player's stockpile, waiting in place and
///   walking again when the way there is shut. A load of some other
///   kind is never delivered: it is wasted at the first transfer instead — a
///   wood-laden worker sent to gold walks straight to the gold and the wood
///   is gone the moment the gold is in hand.
/// - **Harvest** otherwise: walks to the source, takes it up as the carrier's
///   declared presence for the kind allows — waiting in place while a source it
///   cannot share is worked — and works for the source's harvest time, then
///   transfers up to a full load: into its hands when the kind is carried, or
///   straight into the owner's stockpile when it is banked directly, in which
///   case the carrier stays at the source and works the next load. The source
///   loses at most the kind's drain per load; a depleted source is destroyed or
///   left empty on the map, per its [`DepletionPolicy`].
///
/// The order is locked to one resource kind — the first load or source it
/// touches — and never drifts to another. A source the carrier cannot reach is
/// swapped for a nearby one of the same kind, or waited out in place when it is
/// the only one around. The loop ends when no source of the order's kind is
/// left and nothing is carried, there is nowhere at all to deliver the carried
/// load to, or others sat down on the carrier since the order started and the
/// kind's presence would take it off the map from under them.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let Some(mut harvest_component) = world.entity_mut(entity).take::<HarvestComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    let result = advance(entity, order, &mut harvest_component, world);

    // Whichever way the order ends, it ends here — one place stops the work and gives
    // the source back up, so no way out of [`advance`] has to remember to.
    match result.state {
        OrderState::Finished => end_trip(world, entity, &mut harvest_component),
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
        .expect("a Harvest order runs on a carrier")
        .clone();

    // Standing out a blocked path: the retry timer runs down before the
    // carrier looks at the map again.
    if harvest_component.wait > 0 {
        harvest_component.wait -= 1;
        return Processing::state(OrderState::InProcessing);
    }

    let (carried_kind, carried_amount) = {
        let carrier = world
            .entity(entity)
            .get::<ResourceCarrierComponent>()
            .expect("a carrier keeps its load");
        (carrier.kind.clone(), carrier.amount)
    };

    let source = resolve_source(entity, order, harvest_component, &carrier_def, world);

    // A trip whose source no longer qualifies — vanished mid-work, or drained
    // while the carrier sat at it — is abandoned: the carrier stops working and
    // gives the source up, and nothing else happens until it stands on the map
    // again.
    if let Some(harvesting_id) = harvest_component.harvesting
        && source != Some(harvesting_id)
    {
        end_trip(world, entity, harvest_component);
        if entity_def::hidden(world, entity) {
            return Processing::state(OrderState::InProcessing);
        }
    }

    // A load of some other kind than the order's counts for nothing here: it
    // is never banked — the first transfer below replaces it — so a
    // wood-laden worker sent to gold walks straight to the gold, no storage
    // detour.
    let carrying =
        carried_amount > 0 && carried_kind.as_deref() == Some(harvest_component.kind.as_str());
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
            .is_some_and(|target| entity_def::of(world, target).resource_storage.is_some());
    let deliver = carrying
        && (carried_amount >= carried_capacity || ordered_storage_pending || source.is_none());

    if deliver {
        let kind = carried_kind.expect("carrying implies a resource kind");

        // A delivery interrupting a partial trip ends it: the carrier cannot walk a
        // load anywhere while it is still at work in a seam.
        end_trip(world, entity, harvest_component);
        if entity_def::hidden(world, entity) {
            return Processing::state(OrderState::InProcessing);
        }

        let Some(player) = entity_def::owner(world, entity) else {
            return Processing::state(OrderState::Finished);
        };
        let Some(storage) = resolve_storage(entity, order, &kind, player, world) else {
            return Processing::state(OrderState::Finished);
        };

        match chase::advance_to_entity(
            &mut harvest_component.last_chase,
            world,
            entity,
            storage,
            DELIVERY_DISTANCE,
        ) {
            Destination::OutOfReach => {
                // The way to the storage is shut, not the trip over: stand
                // here and walk it again, as a blocked source is waited out
                // below. Ending the order instead would leave the carrier
                // holding a load it will never put down, and nothing puts a
                // carrier back to work — the crowd it could not push through
                // clears on its own, and standing costs only the wait.
                harvest_component.last_chase = None;
                harvest_component.wait = BLOCKED_WALK_RETRY_PERIOD;
                return Processing::state(OrderState::InProcessing);
            }
            Destination::Walk(move_order) => return Processing::suspend(move_order),
            Destination::Arrived => {}
        }

        chase::face_entity(world, entity, storage);
        let banked_at = entity_def::simulation_id(world, storage);
        resources::credit_gathered(world, player, &kind, carried_amount, banked_at);
        let mut entity_mut = world.entity_mut(entity);
        let mut carrier = entity_mut
            .get_mut::<ResourceCarrierComponent>()
            .expect("a carrier keeps its load");
        carrier.kind = None;
        carrier.amount = 0;
        harvest_component.delivered_initial_load = true;

        return Processing::state(OrderState::InProcessing);
    }

    let Some(source_id) = source else {
        // Nothing carried and no source of the order's kind left anywhere near.
        return Processing::state(OrderState::Finished);
    };
    let source_entity = world
        .resource::<EntityIndex>()
        .alive(source_id)
        .expect("resolve_source returns alive sources");
    harvest_component.source = Some(source_id);

    let (source_kind, depletion) = {
        let source_def = entity_def::of(world, source_entity)
            .resource_source
            .as_ref()
            .expect("resolve_source returns sources");
        (source_def.kind().to_string(), source_def.depletion())
    };
    let harvest_data = carrier_def
        .harvest_data(&source_kind)
        .expect("resolve_source returns carryable kinds");

    // A carrier inside its source stands nowhere to walk from: it is at the
    // source for as long as the trip runs. One at work on the map is chased
    // back into reach if anything moved it.
    let inside_source = harvest_component.harvesting == Some(source_id)
        && match harvest_data.presence() {
            WorkPresence::Hidden { .. } => true,
            WorkPresence::Present { .. } | WorkPresence::Attached(_) => false,
        };
    if !inside_source {
        match chase::advance_to_entity(
            &mut harvest_component.last_chase,
            world,
            entity,
            source_entity,
            entity_def::effective_stat_u32(world, entity, EntityStatId::HARVEST_RANGE),
        ) {
            Destination::OutOfReach => {
                // The way to this source is shut, not the source spent: pick
                // another of the same kind the carrier reaches, around where it
                // is, and with none to pick wait here for the way to open — the
                // order gives up only when no source of its kind is left at all.
                let replacement = nearest_source(
                    world,
                    entity,
                    &carrier_def,
                    &harvest_component.kind,
                    Beside::Container(source_id),
                    &[source_id],
                );
                harvest_component.last_chase = None;
                match replacement {
                    Some(replacement) => harvest_component.source = Some(replacement),
                    None => harvest_component.wait = BLOCKED_WALK_RETRY_PERIOD,
                }
                return Processing::state(OrderState::InProcessing);
            }
            Destination::Walk(move_order) => return Processing::suspend(move_order),
            Destination::Arrived => {}
        }

        chase::face_entity(world, entity, source_entity);
    }

    // Take up the source, waiting in place while one that cannot be shared is
    // worked by somebody else, or every berth of it is taken. A carrier that
    // others sat down on since the order started gives the order up.
    if harvest_component.harvesting != Some(source_id) {
        if source_excludes(world, source_entity, entity, &source_kind)
            || berths::shut(world, source_entity, harvest_data.presence())
        {
            return Processing::state(OrderState::InProcessing);
        }
        match work::requires_free_to_hide(world, entity, harvest_data.presence()) {
            Ok(()) => {}
            Err(_) => return Processing::state(OrderState::Finished),
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
        let available = world
            .entity(source_entity)
            .get::<ResourceSourceComponent>()
            .expect("a source keeps what is left in it")
            .amount;
        // A load of some other kind is wasted here, not banked: the hands
        // take the new kind and drop whatever they held — sending a
        // wood-laden worker to gold costs the wood. A carrier that banks where
        // it stands takes nothing in hand, and whatever it holds stays as it
        // is.
        let kept = match harvest_data.banking() {
            Banking::Carried if carried_kind.as_deref() == Some(source_kind.as_str()) => {
                carried_amount
            }
            Banking::Carried | Banking::Direct => 0,
        };
        let room = harvest_data.capacity().saturating_sub(kept);
        // A trip that drains nothing takes a full load whatever is left; one
        // that drains the source takes no more than the source has.
        let take = match harvest_data.drain() {
            0 => room,
            _ => room.min(available),
        };
        let drained = harvest_data.drain().min(take);

        match harvest_data.banking() {
            Banking::Carried => {
                let mut entity_mut = world.entity_mut(entity);
                let mut carrier = entity_mut
                    .get_mut::<ResourceCarrierComponent>()
                    .expect("a carrier keeps its load");
                carrier.kind = Some(source_kind);
                carrier.amount = kept + take;
            }
            Banking::Direct => {
                if let Some(player) = entity_def::owner(world, entity)
                    && take > 0
                {
                    resources::credit_gathered(world, player, &source_kind, take, source_id);
                }
                harvest_component.progress = 0;
            }
        }

        let remaining = {
            let mut source_mut = world.entity_mut(source_entity);
            let mut source_resources = source_mut
                .get_mut::<ResourceSourceComponent>()
                .expect("a source keeps what is left in it");
            source_resources.amount -= drained;
            source_resources.amount
        };

        match harvest_data.banking() {
            // The load moves on once the carrier is back on the grid; one with
            // no cell to return to waits off the map with its load in hand.
            Banking::Carried => end_trip(world, entity, harvest_component),
            // Nothing moves on: the carrier stays at the source.
            Banking::Direct => {}
        }

        if remaining == 0 {
            match depletion {
                DepletionPolicy::Destroy => {
                    spawn::despawn_entity(world, source_entity, DeathCause::Depleted)
                }
                DepletionPolicy::Persist => {}
            }
        }
    }

    Processing::state(OrderState::InProcessing)
}

/// The [`Reading`] of `order`'s target for `entity` when it may start the
/// order, or why it may not — see [`can_start`].
fn reading(world: &World, entity: Entity, order: &Order) -> Result<Reading, Refusal> {
    let reading = read_target(world, entity, order)?;
    // A delivery never leaves the map; the source the order goes on to is
    // judged where it is taken up.
    if reading.source.is_some() {
        let harvest_data = entity_def::harvest_data(world, entity, &reading.kind)
            .expect("a source reading names a kind the carrier carries");
        work::requires_free_to_hide(world, entity, harvest_data.presence())?;
    }
    Ok(reading)
}

/// The [`Reading`] of `order`'s target for `entity`, or why the target takes
/// no harvest from it.
fn read_target(world: &World, entity: Entity, order: &Order) -> Result<Reading, Refusal> {
    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");
    let Some(carrier_def) = entity_def::of(world, entity).resource_carrier.as_ref() else {
        return Err(Refusal::Incapable);
    };
    orders::requires_operating(world, entity)?;
    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return Err(Refusal::TargetGone);
    };
    orders::target_operating(world, target)?;

    let target_def = entity_def::of(world, target);
    // A source of a kind the carrier works names the order's kind outright —
    // one that admits this carrier: its owner's, when it has an owner, and
    // offering the berths the carrier sits in, when it attaches.
    if world.entity(target).contains::<ResourceSourceComponent>()
        && let Some(source) = target_def.resource_source.as_ref()
        && carrier_def.can_carry(source.kind())
    {
        if !admits(world, target, entity, carrier_def, source.kind()) {
            return Err(Refusal::TargetUnfit);
        }
        return Ok(Reading {
            kind: source.kind().to_string(),
            source: Some(target_id),
        });
    }
    // A storage takes a delivery: the carrier's own, holding a load of a kind
    // the storage accepts.
    let Some(storage) = target_def.resource_storage.as_ref() else {
        return Err(Refusal::TargetUnfit);
    };
    let own = entity_def::same_owner(world, entity, target);
    let load = world
        .entity(entity)
        .get::<ResourceCarrierComponent>()
        .filter(|carrier| carrier.amount > 0)
        .and_then(|carrier| carrier.kind.clone());
    match load {
        Some(kind) if own && storage.accepts(&kind) => Ok(Reading { kind, source: None }),
        Some(_) | None => Err(Refusal::TargetUnfit),
    }
}

/// Whether `source` admits `carrier` for `kind`: a source with an owner takes
/// only that owner's carriers, the carrier's kind must list the source's type,
/// and a carrier that attaches for the kind needs the source to offer the berth
/// group it sits in.
fn admits(
    world: &World,
    source: Entity,
    carrier: Entity,
    carrier_def: &ResourceCarrierDef,
    kind: &str,
) -> bool {
    let owned_by_another = entity_def::owner(world, source)
        .is_some_and(|holder| Some(holder) != entity_def::owner(world, carrier));
    if owned_by_another {
        return false;
    }
    let Some(data) = carrier_def.harvest_data(kind) else {
        return false;
    };
    data.sources().admits(entity_def::of(world, source))
        && data
            .presence()
            .attachment()
            .is_none_or(|attachment| berths::offers(world, source, attachment.berths()))
}

/// Whether `entity` is shut out of working `source` for `kind` by the crew already
/// on it.
fn source_excludes(world: &World, source: Entity, entity: Entity, kind: &str) -> bool {
    crew::excludes::<UnderHarvestComponent>(world, source, entity, |world, carrier| {
        entity_def::harvest_data(world, carrier, kind)
            .expect("every carrier on a source began a trip for the kind it yields")
            .presence()
            .crewing()
    })
}

/// Starts a trip on `source`: the carrier takes the source up, joining its crew, and
/// is at work from now until [`end_trip`].
///
/// Being at work and being off the grid are separate facts — a carrier inside a seam is
/// both — so the mark goes on regardless, and the presence decides only where the
/// carrier stands for the duration.
fn begin_trip(
    world: &mut World,
    entity: Entity,
    source: Entity,
    harvest: &mut HarvestComponent,
    presence: &WorkPresence,
) {
    harvest.harvesting = Some(entity_def::simulation_id(world, source));
    harvest.progress = 0;
    crew::join::<UnderHarvestComponent>(world, source, entity);

    world.entity_mut(entity).insert(HarvestingComponent);
    work::enter(world, entity, presence, source);
}

/// Ends the trip in progress: the carrier stops working and gives the source back
/// up, then comes back onto the grid if it left it for the source — or stays off the
/// grid with its return queued for a later tick when no cell is free.
fn end_trip(world: &mut World, entity: Entity, harvest: &mut HarvestComponent) {
    let beside = beside_source(harvest);
    stop_work(world, entity, harvest);
    work::leave(world, entity, beside);
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

/// Picks the source to harvest from: the trip in progress, the source the order
/// settled on, the ordered target, or the nearest matching source within
/// [`SOURCE_SEARCH_RADIUS`] — in that priority order.
///
/// The settled source outranks the ordered target: it starts as the target and
/// moves only when a replacement is picked, which must then stick.
///
/// Only live sources of the order's own kind, with anything left in them,
/// qualify.
fn resolve_source(
    entity: Entity,
    order: &Order,
    harvest: &HarvestComponent,
    carrier_def: &ResourceCarrierDef,
    world: &World,
) -> Option<SimulationId> {
    let kind = harvest.kind.as_str();

    let target_id = order
        .harvest_target()
        .expect("Harvest order must have a target");

    for candidate in [harvest.harvesting, harvest.source, Some(target_id)]
        .into_iter()
        .flatten()
    {
        if source_matches(world, candidate, entity, carrier_def, kind) {
            return Some(candidate);
        }
    }

    nearest_source(
        world,
        entity,
        carrier_def,
        kind,
        beside_source(harvest),
        &[],
    )
}

/// The nearest source of `kind` but `except` within [`SOURCE_SEARCH_RADIUS`]
/// of where `entity` searches from — beside `beside`, off the map — that
/// [`source_matches`] and that it reaches within its harvest range.
fn nearest_source(
    world: &World,
    entity: Entity,
    carrier_def: &ResourceCarrierDef,
    kind: &str,
    beside: Beside,
    except: &[SimulationId],
) -> Option<SimulationId> {
    let approach = approach(world, entity, beside);
    let range = entity_def::effective_stat_u32(world, entity, EntityStatId::HARVEST_RANGE);
    nearest(
        world,
        searching_from(world, entity, beside),
        Some(SOURCE_SEARCH_RADIUS),
        |id, candidate| {
            !except.contains(&id)
                && source_matches(world, id, entity, carrier_def, kind)
                && approach.reaches(world, candidate, range)
        },
    )
}

/// Whether `id` is a live, risen source of `kind`, with anything left in it,
/// that admits `carrier` and that it can carry from.
fn source_matches(
    world: &World,
    id: SimulationId,
    carrier: Entity,
    carrier_def: &ResourceCarrierDef,
    kind: &str,
) -> bool {
    let Some(source) = world.resource::<EntityIndex>().interactable(world, id) else {
        return false;
    };
    let Some(source_def) = entity_def::of(world, source).resource_source.as_ref() else {
        return false;
    };
    let risen = match entity_def::operation(world, source) {
        // One still rising already holds the amount of the source it covers;
        // it becomes a source of its own only once it has risen.
        Operation::Rising => false,
        Operation::Operating | Operation::Disabled(_) => true,
        Operation::Stranded => unreachable!("a stranded body is hidden"),
    };
    risen
        && world
            .entity(source)
            .get::<ResourceSourceComponent>()
            .is_some_and(|left| left.amount > 0)
        && carrier_def.can_carry(source_def.kind())
        && source_def.kind() == kind
        && admits(world, source, carrier, carrier_def, kind)
}

/// Picks the storage to deliver to: the ordered target if it qualifies, otherwise
/// the nearest operating storage of the owner that accepts `kind`.
fn resolve_storage(
    entity: Entity,
    order: &Order,
    kind: &str,
    player: PlayerId,
    world: &World,
) -> Option<Entity> {
    let qualifies = |storage: Entity| -> bool {
        entity_def::of(world, storage)
            .resource_storage
            .as_ref()
            .is_some_and(|storage_def| storage_def.accepts(kind))
            && entity_def::owner(world, storage) == Some(player)
            && orders::target_operating(world, storage).is_ok()
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

    // A carrier delivers standing: its trip ended before the search.
    let approach = Approach::from_where_it_is(world, entity);
    let id = nearest(
        world,
        entity_def::standing_rect(world, entity),
        None,
        |_, candidate| {
            qualifies(candidate) && approach.reaches(world, candidate, DELIVERY_DISTANCE)
        },
    )?;
    world.resource::<EntityIndex>().alive(id)
}

/// The cells a carrier searches from: the ones it stands on, per
/// [`entity_def::standing_rect`], or, off the map, the footprint it would be
/// set down beside.
fn searching_from(world: &World, entity: Entity, beside: Beside) -> CellRect {
    if entity_def::hidden(world, entity) {
        entity_def::beside_rect(world, entity, beside)
    } else {
        entity_def::standing_rect(world, entity)
    }
}

/// How the carrier gets within reach of what it searches for: from where it
/// stands, or, off the map, from where it would be set down beside `beside`.
fn approach(world: &World, entity: Entity, beside: Beside) -> Approach {
    if entity_def::hidden(world, entity) {
        Approach::coming_from(
            world,
            entity,
            ComesFrom::steps_out_of(world, entity, beside),
        )
    } else {
        Approach::from_where_it_is(world, entity)
    }
}

/// What a carrier off the map is set down beside: the source it works, or
/// where it went in when it works none.
fn beside_source(harvest: &HarvestComponent) -> Beside {
    match harvest.harvesting {
        Some(source) => Beside::Container(source),
        None => Beside::WentIn,
    }
}

/// Finds the alive entity matching `filter` nearest to `from`, measured to the
/// cells the entity stands on ([`reach::distance_to`]) — so `from` is the
/// searcher's [`searching_from`], the side that does the reaching.
/// Ties break on the lower [`SimulationId`], so the result is deterministic.
fn nearest(
    world: &World,
    from: CellRect,
    max_distance: Option<u32>,
    filter: impl Fn(SimulationId, Entity) -> bool,
) -> Option<SimulationId> {
    let projection = world.resource::<Map>().projection();
    let mut best: Option<(u32, SimulationId)> = None;

    for (id, entity) in world.resource::<EntityIndex>().alive_entries() {
        // Nothing off the map is reached; the distance gate comes before the
        // filter, which may judge a walk there.
        if entity_def::hidden(world, entity) {
            continue;
        }
        if let Some(max) = max_distance
            && !projection.in_range_for_rects(from, entity_def::standing_rect(world, entity), max)
        {
            continue;
        }
        if !filter(id, entity) {
            continue;
        }
        // Footprint to footprint: an anchor is not the body, and a wide
        // seeker's nearest edge may rank the candidates differently.
        let distance = reach::distance_to(world, from, entity);
        // Ascending id iteration: strictly-closer wins, ties keep the lower id.
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, id));
        }
    }

    best.map(|(_, id)| id)
}
