//! Changing what an entity *is*: one type in place of another, on the same
//! entity.
//!
//! The entity survives, so everything not derived from its type comes along
//! untouched — its id, its selection, its order queue, its buffs, and whatever
//! it was carrying. What the type owns is re-fitted: where it stands on the
//! grid, its stat bases, its pools, and its capability components.
//!
//! Who may become what, how long it takes, what it costs, when the ground is
//! secured, and whether it can be called off are all terms of the
//! [`MorphTransition`] the entity's own type declares. The order settles those
//! terms at [`prepare`] and lands the change when its progress runs out; a
//! change that cannot happen finishes silently, like any other order that
//! finds its work impossible.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use crate::{
    components::{
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        morph::{MorphComponent, MorphReservation},
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        transport::TransporterComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{EventRecord, SimulationEvent, SpendCause},
    game_loop::cast_cost,
    map::{Map, OccupancyClass},
    movement_model::MovementModel,
    order::Order,
    requirements,
    session::player_slot::PlayerId,
    spawn,
};
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeId,
    location::LocationDef,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    registry::ContentRegistry,
};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_physics::body;

/// Whether ordering `entity` into `type_name` is worth queueing at all: the
/// entity's own type must declare the transition, the player must meet its
/// requirements, and the destination must be able to seat whatever is aboard.
///
/// Deliberately silent about costs and whether the footprint fits, because
/// both are only knowable when the order actually starts.
pub fn would_start(world: &World, player: PlayerId, entity: Entity, type_name: &str) -> bool {
    let Some(transition) = transition_into(world, entity, type_name) else {
        return false;
    };
    requirements::met(world, player, transition.requires())
        && destination(world, type_name)
            .is_some_and(|(type_id, _)| cargo_fits(world, entity, type_id))
}

/// Called once when a Morph order becomes the front `New` entry.
///
/// Settles every term the transition declares: refuses outright what could
/// never work (a destination the type declares no transition into, cargo that
/// would not fit, requirements no longer met, a cost that cannot be paid), and
/// only then commits — a reserving transition claims its destination footprint
/// or refuses on the spot, and the cost is drawn. A revalidating transition
/// touches no ground early; whether its footprint fits is decided when the
/// change lands, because the ground can be taken while the unit is still
/// turning into something.
///
/// While the change runs the unit keeps its old form entirely — layer,
/// footprint, and answerability — and takes the new one only when the progress
/// lands. It is helpless for the window either way, since the order occupies
/// its queue.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Order::Morph { type_name } = order else {
        unreachable!("prepare called with a non-Morph order");
    };

    let Some(transition) = transition_into(world, entity, type_name) else {
        return OrderState::Finished;
    };
    let Some((type_id, _)) = destination(world, type_name) else {
        return OrderState::Finished;
    };
    let Some(player) = transition_payer(world, entity) else {
        return OrderState::Finished;
    };
    if !cargo_fits(world, entity, type_id)
        || !requirements::met(world, player, transition.requires())
        || !cast_cost::can_pay(world, entity, player, transition.costs())
    {
        return OrderState::Finished;
    }

    let time = morph_time(world, entity, transition.time());
    if time == 0 {
        // Nothing to wait for: paid and landed in one breath, in the order
        // the timed path keeps — the cost leaves the old form's pools before
        // the landing rescales them, and a fizzled refundable change gives
        // the payment back exactly as a timed landing does.
        cast_cost::pay(
            world,
            entity,
            player,
            transition.costs(),
            SpendCause::Morph {
                entity: entity_def::simulation_id(world, entity),
            },
        );
        if !land(world, entity, type_name, true)
            && let MorphCancel::Refundable = transition.cancel()
        {
            cast_cost::refund(
                world,
                entity,
                player,
                transition.costs(),
                SpendCause::Morph {
                    entity: entity_def::simulation_id(world, entity),
                },
            );
        }
        return OrderState::Finished;
    }

    let morph = match transition.placement() {
        MorphPlacement::Reserve => match reserve(world, entity, type_name) {
            Some(morph) => morph,
            None => return OrderState::Finished,
        },
        MorphPlacement::Revalidate => MorphComponent {
            type_name: type_name.clone(),
            progress: 0,
            reservation: None,
        },
    };
    cast_cost::pay(
        world,
        entity,
        player,
        transition.costs(),
        SpendCause::Morph {
            entity: entity_def::simulation_id(world, entity),
        },
    );
    world.entity_mut(entity).insert(morph);
    OrderState::InProcessing
}

/// Called for every Morph entry that a cancel reaches, judged by the
/// transition's own cancel terms.
///
/// A **committed** transition refuses a soft cancel — the window is a real
/// decision, not a feint a player could bait with and think better of — and
/// honors only force, because force is not the player changing their mind: it
/// is the engine flushing the queue for something that overrides everything
/// (dying, being taken aboard), and the payment is lost with the change. A
/// **forfeiting** transition can be called off but keeps the payment. A
/// **refundable** one gives the full cost back. Any early end lets go of the
/// ground a reserving change held.
pub fn cancel_processing(
    entity: Entity,
    order: &Order,
    policy: CancelPolicy,
    entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    let Order::Morph { type_name } = order else {
        unreachable!("cancel_processing called with a non-Morph order");
    };

    let cancel = transition_into(world, entity, type_name)
        .map(|transition| transition.cancel())
        // The transition existed when the order started; content does not
        // change mid-session, so this is only a guard against a component
        // outliving its order.
        .unwrap_or(MorphCancel::Forfeit);
    let refundable = match (cancel, policy) {
        // A committed window refuses the player's cancel only once it is
        // actually open: an entry still queued has taken nothing and
        // promised nothing, and drops like any other.
        (MorphCancel::Committed, CancelPolicy::Soft) => {
            if entry_state == OrderState::InProcessing {
                return OrderState::InProcessing;
            }
            false
        }
        (MorphCancel::Committed, CancelPolicy::Force) => false,
        (MorphCancel::Forfeit, _) => false,
        (MorphCancel::Refundable, _) => true,
    };

    if let Some(morph) = world.entity_mut(entity).take::<MorphComponent>() {
        release_reservation(world, &morph);
        if refundable
            && let Some(transition) = transition_into(world, entity, type_name)
            && let Some(player) = transition_payer(world, entity)
        {
            cast_cost::refund(
                world,
                entity,
                player,
                transition.costs(),
                SpendCause::Morph {
                    entity: entity_def::simulation_id(world, entity),
                },
            );
        }
    }
    OrderState::Finished
}

/// Advance a Morph order by one tick.
///
/// The unit does nothing else while the progress runs; when it runs out the
/// change lands on the transition's own terms: a reserving change lets go of
/// its reservation and takes the ground it held without asking again, a
/// revalidating one re-checks the destination and is refused if the ground was
/// taken — refunding a refundable transition's cost, since the change never
/// happened.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> OrderState {
    let Some(mut morph) = world.entity_mut(entity).take::<MorphComponent>() else {
        return OrderState::Finished;
    };
    let Some(transition) = transition_into(world, entity, &morph.type_name) else {
        release_reservation(world, &morph);
        return OrderState::Finished;
    };
    let time = morph_time(world, entity, transition.time());

    morph.progress += 1;
    if morph.progress < time {
        world.entity_mut(entity).insert(morph);
        return OrderState::InProcessing;
    }

    // Between the release and the landing nothing else runs, so the ground a
    // reservation held passes to the new footprint atomically.
    release_reservation(world, &morph);
    let revalidate = match transition.placement() {
        MorphPlacement::Reserve => false,
        MorphPlacement::Revalidate => true,
    };
    if !land(world, entity, &morph.type_name, revalidate)
        && let MorphCancel::Refundable = transition.cancel()
        && let Some(player) = transition_payer(world, entity)
    {
        // The change fizzled: the entity stays as it was, so a refundable
        // transition's payment goes back the same way a cancel returns it.
        cast_cost::refund(
            world,
            entity,
            player,
            transition.costs(),
            SpendCause::Morph {
                entity: entity_def::simulation_id(world, entity),
            },
        );
    }
    OrderState::Finished
}

//
// ─── Landing the change ─────────────────────────────────────────────────────────
//

/// The landing: rewrites the entity's type in place, or returns `false` with
/// everything untouched when the change cannot happen — a destination that is
/// unregistered or that the entity's own type declares no transition into
/// (the vocabulary is the authority: a command merely names a destination,
/// and anything undeclared is refused, or any wire-legal command could turn
/// any unit into any registered type), a cell-model mover caught between
/// cells (its anchor names a cell it only partly holds, so there is no
/// settled footprint to swap), cargo the new form cannot seat, or a
/// destination footprint that no longer fits.
///
/// Whether the footprint is re-tested is the caller's `revalidate`: a
/// reserving completion skips the check because it holds the ground already.
fn land(world: &mut World, entity: Entity, type_name: &str, revalidate: bool) -> bool {
    let Some((type_id, to)) = destination(world, type_name) else {
        return false;
    };
    if transition_into(world, entity, type_name).is_none() {
        return false;
    }
    let Some(from) = entity_def::of(world, entity).location else {
        return false;
    };
    let Some(position) = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.position)
    else {
        return false;
    };

    if let MovementModel::Cell = world.resource::<Map>().movement_model()
        && super::movement::is_mid_crossing(position)
    {
        return false;
    }
    if !cargo_fits(world, entity, type_id) {
        return false;
    }

    // The footprint is anchored at its origin, so a size change recentres it:
    // growing from the same corner would shift the unit's middle sideways.
    let anchor = recentred(position, from, to);
    if !reoccupy(
        world, entity, position, anchor, from, to, type_id, revalidate,
    ) {
        return false;
    }

    // Identity stays; the type is rewritten under it, and the anchor follows.
    let base_stats = world
        .resource::<ContentRegistry>()
        .def(type_id)
        .base_stats
        .clone();
    let morphed_from = entity_def::type_id(world, entity);
    if let Some(mut info) = world.entity_mut(entity).get_mut::<EntityInfoComponent>() {
        info.become_type(type_id, type_name);
    }
    if let Some(mut location) = world.entity_mut(entity).get_mut::<LocationComponent>() {
        location.position = anchor;
    }

    // Pools carry their *proportion* across, read before the bases move under
    // them: a form with a different maximum keeps a full unit full and a
    // half-dead one half-dead, rather than keeping an absolute value that means
    // something else on the other side.
    let health = filled_fraction(
        world
            .entity(entity)
            .get::<HealthComponent>()
            .map(|health| health.current()),
        entity_def::effective_stat(world, entity, EntityStatId::MAX_HEALTH),
    );
    let energy = filled_fraction(
        world
            .entity(entity)
            .get::<EnergyComponent>()
            .map(|energy| energy.current()),
        entity_def::effective_stat(world, entity, EntityStatId::MAX_ENERGY),
    );

    spawn::seed_stats(world, entity, &base_stats);
    spawn::fit_components(world, entity, type_id);

    // The pools are re-fitted to what the destination declares: a form with
    // the stat keeps the carried proportion — or starts full when the old
    // form had no such pool — and a form without it loses the pool component
    // outright, because a zero-maximum pool would read as dead rather than
    // as poolless.
    match base_stats.get(&EntityStatId::MAX_HEALTH) {
        Some(&max) => {
            let filled = max * health.unwrap_or(FixedU64::ONE);
            let mut entity_mut = world.entity_mut(entity);
            if let Some(mut pool) = entity_mut.get_mut::<HealthComponent>() {
                *pool = HealthComponent::full(filled);
            } else {
                entity_mut.insert(HealthComponent::full(filled));
            }
        }
        None => {
            world.entity_mut(entity).remove::<HealthComponent>();
        }
    }
    match base_stats.get(&EntityStatId::MAX_ENERGY) {
        Some(&max) => {
            let filled = max * energy.unwrap_or(FixedU64::ONE);
            let mut entity_mut = world.entity_mut(entity);
            if let Some(mut pool) = entity_mut.get_mut::<EnergyComponent>() {
                *pool = EnergyComponent::full(filled);
            } else {
                entity_mut.insert(EnergyComponent::full(filled));
            }
        }
        None => {
            world.entity_mut(entity).remove::<EnergyComponent>();
        }
    }

    // The plan was made against the old form's layers and clearance, so it means
    // nothing now. The order queue stays: a unit told to go somewhere and then
    // changed form still wants to go there, by whatever way its new form travels.
    if let Some(mut movement) = world.entity_mut(entity).get_mut::<MoveComponent>() {
        movement.repath_avoiding_claims();
    }

    // Announced only once the transition has committed: a refused reoccupation
    // returns above, and a morph that did not happen is not one to report.
    let announced = SimulationEvent::EntityMorphed {
        entity: entity_def::simulation_id(world, entity),
        from: morphed_from,
    };
    world.resource_mut::<EventRecord>().emit(announced);

    true
}

/// Moves the entity's presence on the grid from the old footprint to the new
/// one, or returns `false` with the old footprint intact when the new one no
/// longer fits.
///
/// The old footprint comes off *before* the new one is tested, so a form's own
/// presence never reads as something standing in its way — without that, a
/// same-layer change would always refuse. A static footprint is displaced and
/// put back; a mover's claim is lifted from where the claim plane actually
/// holds it — the rounded anchor the continuous rebuild stamps, which is the
/// settled cell itself under the cell model — because displacing a claim is a
/// no-op under the continuous model (its clears belong to the rebuild alone)
/// and the put-back would mint bits the plane never held. A refused change
/// restores exactly what came off. A caller that secured the ground in
/// advance passes `revalidate = false` and the swap is unconditional.
#[allow(clippy::too_many_arguments)]
fn reoccupy(
    world: &mut World,
    entity: Entity,
    position: FixedUVec2,
    anchor: FixedUVec2,
    from: LocationDef,
    to: LocationDef,
    type_id: EntityTypeId,
    revalidate: bool,
) -> bool {
    // A hidden entity holds no cells at all, so there is nothing to move.
    if world.entity(entity).contains::<HiddenComponent>() {
        return true;
    }
    let old_class = OccupancyClass::of(entity_def::of(world, entity));
    let new_class = OccupancyClass::of(world.resource::<ContentRegistry>().def(type_id));
    // Unchanged presence needs no check and cannot fail: the same cells stay
    // marked on the same plane. Anything else — cells, plane, or solidity —
    // has to swap.
    if from.occupation() == to.occupation()
        && from.size() == to.size()
        && old_class == new_class
        && from.solidity() == to.solidity()
    {
        return true;
    }

    let facing = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.facing)
        .unwrap_or(spawn::DEFAULT_FACING);
    let standing = LocationComponent::new(position, facing);
    let placed = LocationComponent::new(anchor, facing);

    let mut map = world.resource_mut::<Map>();
    let own = lift_standing_presence(&mut map, &standing, &from, old_class);
    if revalidate && !map.can_place_entity(&placed, &to) {
        restore_standing_presence(&mut map, &standing, &from, old_class, &own);
        return false;
    }
    map.place_entity(&placed, &to, new_class);
    true
}

/// Takes the entity's standing presence off the grid ahead of testing or
/// taking its destination, returning the claim cells actually lifted so
/// [`restore_standing_presence`] can put back exactly those.
fn lift_standing_presence(
    map: &mut Map,
    standing: &LocationComponent,
    from: &LocationDef,
    class: OccupancyClass,
) -> Vec<CellPos> {
    match class {
        OccupancyClass::Static => {
            map.displace_entity(standing, from, class);
            Vec::new()
        }
        OccupancyClass::Claim => {
            if !from.solidity().claims_cells() {
                return Vec::new();
            }
            map.take_claim(
                from.occupation(),
                body::anchor(standing.position),
                from.size(),
            )
        }
    }
}

/// Puts back what [`lift_standing_presence`] took.
fn restore_standing_presence(
    map: &mut Map,
    standing: &LocationComponent,
    from: &LocationDef,
    class: OccupancyClass,
    own: &[CellPos],
) {
    match class {
        OccupancyClass::Static => map.place_entity(standing, from, class),
        OccupancyClass::Claim => map.restore_claim(from.occupation(), own),
    }
}

/// Whether the destination form can seat what the entity is carrying: it must
/// admit each passenger by type or tag, and have the slots for all of them.
fn cargo_fits(world: &World, entity: Entity, type_id: EntityTypeId) -> bool {
    let aboard: Vec<Entity> = world
        .entity(entity)
        .get::<TransporterComponent>()
        .map(|transporter| {
            transporter
                .passengers
                .iter()
                .filter_map(|&id| world.resource::<EntityIndex>().alive(id))
                .collect()
        })
        .unwrap_or_default();
    if aboard.is_empty() {
        return true;
    }

    let def = world.resource::<ContentRegistry>().def(type_id);
    let Some(transporter) = def.transporter.as_ref() else {
        return false;
    };
    let capacity = def
        .base_stat(EntityStatId::CARGO_CAPACITY)
        .map(|capacity| capacity.to_num::<u32>())
        .unwrap_or(0);

    let mut slots = 0;
    for passenger in aboard {
        let passenger_def = entity_def::of(world, passenger);
        // Admission is by type name or tag, exactly as boarding checks it: a form
        // that would not have let this passenger in cannot inherit it either.
        if !transporter.admits(&passenger_def.name, |tag| passenger_def.tags.contains(tag)) {
            return false;
        }
        slots += entity_def::effective_stat_u32(world, passenger, EntityStatId::CARGO_SIZE);
    }
    capacity >= slots
}

/// The destination type's handle and location, or `None` when it is not
/// something an entity could stand as.
fn destination(world: &World, type_name: &str) -> Option<(EntityTypeId, LocationDef)> {
    let registry = world.resource::<ContentRegistry>();
    let type_id = registry.type_id(type_name)?;
    Some((type_id, registry.def(type_id).location?))
}

/// The anchor a footprint of the destination's size takes so that its middle
/// stays where the old one's was.
fn recentred(position: FixedUVec2, from: LocationDef, to: LocationDef) -> FixedUVec2 {
    let shift = |value: FixedU64, old: u32, new: u32| {
        let half = |cells: u32| FixedU64::from_num(cells) / 2;
        // Saturating, because a footprint recentred at the grid's edge would
        // otherwise run off it; placement then refuses on its own terms.
        if new >= old {
            value.saturating_sub(half(new) - half(old))
        } else {
            value + (half(old) - half(new))
        }
    };
    FixedUVec2::new(
        shift(position.x, from.size().width, to.size().width),
        shift(position.y, from.size().height, to.size().height),
    )
}

/// How full a pool is, as a fraction of its maximum, or `None` when there is no
/// pool to carry over.
fn filled_fraction(current: Option<FixedU64>, maximum: Option<FixedU64>) -> Option<FixedU64> {
    let (current, maximum) = current.zip(maximum)?;
    (maximum > FixedU64::ZERO).then(|| current / maximum)
}

//
// ─── Securing the ground ────────────────────────────────────────────────────────
//

/// Claims the destination footprint ahead of the change, or returns `None`
/// when it cannot be secured — the ground is taken, or a cell-model mover is
/// between cells with no settled footprint to reserve around. The reservation
/// reads as occupied to placement, spawning, and claim-honoring movement for
/// the whole window, so completion finds the ground it was promised. Cells
/// the entity's own standing claim already covers stay under that claim — the
/// reservation records exactly what it took, and releasing it gives back
/// exactly that.
fn reserve(world: &mut World, entity: Entity, type_name: &str) -> Option<MorphComponent> {
    let (_, to) = destination(world, type_name)?;
    let from = entity_def::of(world, entity).location?;
    let position = world.entity(entity).get::<LocationComponent>()?.position;
    // A mover between cells has no settled footprint to reserve around.
    if let MovementModel::Cell = world.resource::<Map>().movement_model()
        && super::movement::is_mid_crossing(position)
    {
        return None;
    }
    // A hidden entity holds no cells and will land through the same exemption,
    // so there is nothing to secure.
    if world.entity(entity).contains::<HiddenComponent>() {
        return Some(MorphComponent {
            type_name: type_name.to_string(),
            progress: 0,
            reservation: None,
        });
    }

    let facing = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.facing)
        .unwrap_or(spawn::DEFAULT_FACING);
    let anchor = recentred(position, from, to);
    let standing = LocationComponent::new(position, facing);
    let placed = LocationComponent::new(anchor, facing);
    let old_class = OccupancyClass::of(entity_def::of(world, entity));

    // The entity's own presence comes off before the destination is tested,
    // exactly as the landing itself will do, and goes straight back either way.
    let mut map = world.resource_mut::<Map>();
    let own = lift_standing_presence(&mut map, &standing, &from, old_class);
    let fits = map.can_place_entity(&placed, &to);
    restore_standing_presence(&mut map, &standing, &from, old_class, &own);
    if !fits {
        return None;
    }

    let cells = map.reserve_claim(to.occupation(), CellPos::from(anchor), to.size());
    Some(MorphComponent {
        type_name: type_name.to_string(),
        progress: 0,
        reservation: Some(MorphReservation {
            cells,
            mask: to.occupation(),
        }),
    })
}

/// Lets go of the ground a reserving change held.
fn release_reservation(world: &mut World, morph: &MorphComponent) {
    let Some(reservation) = &morph.reservation else {
        return;
    };
    world
        .resource_mut::<Map>()
        .release_claim(reservation.mask, &reservation.cells);
}

//
// ─── The transition's terms ─────────────────────────────────────────────────────
//

/// The transition `entity`'s own type declares into `type_name`, cloned so a
/// caller holding the world mutably can keep the terms in hand.
///
/// This is the authority on who may become what: the command merely names a
/// destination, and anything the vocabulary did not declare is refused —
/// otherwise any wire-legal command could turn any unit into any registered
/// type.
fn transition_into(world: &World, entity: Entity, type_name: &str) -> Option<MorphTransition> {
    entity_def::of(world, entity)
        .morphs
        .iter()
        .find(|transition| transition.into_type() == type_name)
        .cloned()
}

/// Ticks the change takes for this entity, under the transition's own terms: a
/// constant is what it says, and a stat names the changing entity's
/// **effective** value — so buffs and researches move it, and it is re-read
/// every tick while the change runs. A time of zero lands the tick it starts.
fn morph_time(world: &World, entity: Entity, time: MorphTime) -> u32 {
    match time {
        MorphTime::Constant(ticks) => ticks,
        MorphTime::Stat(id) => entity_def::effective_stat(world, entity, id)
            .map(|time| time.to_num::<u32>())
            .unwrap_or(0),
    }
}

/// The player whose stockpile the transition draws from and refunds to: the
/// changing entity's owner.
fn transition_payer(world: &World, entity: Entity) -> Option<PlayerId> {
    world
        .entity(entity)
        .get::<OwnerComponent>()
        .map(|owner| owner.player())
}
