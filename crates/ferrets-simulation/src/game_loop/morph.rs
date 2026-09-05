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
//! finds its work impossible. A transition may name an interim form, worn from
//! the start of the change until it lands, and returned from when the change
//! ends early.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use super::orders::{self, Processing, Refusal};
use crate::{
    components::{
        dying::DyingComponent,
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        morph::{MorphComponent, MorphReservation},
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderState},
        transport::TransporterComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{EventRecord, SimulationEvent, SpendCause},
    fields,
    game_loop::cast_cost,
    map::{Map, OccupancyClass},
    movement_model::MovementModel,
    order::Order,
    requirements,
    session::player_id::PlayerId,
    spawn::{self, FieldReach, StandingActs},
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

/// Whether `entity` may start this Morph: its type declares the transition,
/// the destination exists and seats whatever is aboard, and it operates.
/// Requirements, the cost and the ground are settled when the order starts.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let Order::Morph { type_name } = order else {
        unreachable!("can_start called with a non-Morph order");
    };
    if transition_into(world, entity, type_name).is_none() {
        return Err(Refusal::Incapable);
    }
    let Some((type_id, _)) = destination(world, type_name) else {
        return Err(Refusal::Incapable);
    };
    if !cargo_fits(world, entity, type_id) {
        return Err(Refusal::TargetUnfit);
    }
    orders::requires_operating(world, entity)
}

/// Whether `player` meets the requirements of `entity`'s transition into
/// `type_name`. A transition the type does not declare has none to meet.
pub fn requirements_met(world: &World, player: PlayerId, entity: Entity, type_name: &str) -> bool {
    transition_into(world, entity, type_name)
        .is_none_or(|transition| requirements::met(world, player, transition.requires()))
}

/// Called once when a Morph order becomes the front `New` entry.
///
/// Settles every term the transition declares: refuses outright what cannot
/// start (see [`can_start`]), requirements no longer met, or a cost that cannot
/// be paid, and only then commits — a reserving transition claims its destination footprint
/// or refuses on the spot, and the cost is drawn. A revalidating transition
/// touches no ground early; whether its footprint fits is decided when the
/// change lands.
///
/// While the change runs the unit wears its interim form, when the transition
/// declares one, or keeps its old form, and takes the new one only when the
/// progress lands. The order occupies its queue for the window either way.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Order::Morph { type_name } = order else {
        unreachable!("prepare called with a non-Morph order");
    };

    if can_start(world, entity, order).is_err() {
        return OrderState::Finished;
    }
    let transition =
        transition_into(world, entity, type_name).expect("can_start found the transition");
    let Some(player) = entity_def::owner(world, entity) else {
        return OrderState::Finished;
    };
    if !requirements::met(world, player, transition.requires())
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
        if !land(world, entity, type_name, Landing::Revalidated)
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
            from: entity_def::type_id(world, entity),
            into: type_name.clone(),
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
    // The interim form is put on as the change starts; a form that cannot be
    // worn — it will not seat what is aboard, or its ground is refused — ends
    // the change before it began, on the same refund terms as a fizzle.
    if let Some(via) = transition.via_type()
        && !land(world, entity, via, Landing::Reserved)
    {
        release_reservation(world, &morph);
        if let MorphCancel::Refundable = transition.cancel() {
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
    // A queued entry was never prepared: nothing was paid or reserved for it,
    // and the change on the entity, if any, is the one under way in front.
    match entry_state {
        OrderState::New => return OrderState::Finished,
        OrderState::InProcessing | OrderState::Suspended => {}
        OrderState::Finished => unreachable!("Finished entries never stay in the queue"),
    }

    let under_way = world.entity(entity).get::<MorphComponent>().cloned();
    let transition = match &under_way {
        Some(morph) => terms(world, morph),
        None => transition_into(world, entity, type_name),
    };
    let cancel = transition
        .as_ref()
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
        // A dying entity keeps the form its death was announced in.
        if !world.entity(entity).contains::<DyingComponent>() {
            return_to_origin(world, entity, &morph);
        }
        if refundable
            && let Some(transition) = transition
            && let Some(player) = entity_def::owner(world, entity)
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
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut morph) = world.entity_mut(entity).take::<MorphComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let Some(transition) = terms(world, &morph) else {
        release_reservation(world, &morph);
        return Processing::state(OrderState::Finished);
    };
    let time = morph_time(world, entity, transition.time());

    morph.progress += 1;
    if morph.progress < time {
        world.entity_mut(entity).insert(morph);
        return Processing::state(OrderState::InProcessing);
    }

    // Between the release and the landing nothing else runs, so the ground a
    // reservation held passes to the new footprint atomically.
    release_reservation(world, &morph);
    let landing = match transition.placement() {
        MorphPlacement::Reserve => Landing::Reserved,
        MorphPlacement::Revalidate => Landing::Revalidated,
    };
    if !land(world, entity, &morph.into, landing) {
        // The change fizzled: the entity returns to what it was, so a
        // refundable transition's payment goes back the same way a cancel
        // returns it.
        return_to_origin(world, entity, &morph);
    }
    if !land_succeeded(world, entity, &morph)
        && let MorphCancel::Refundable = transition.cancel()
        && let Some(player) = entity_def::owner(world, entity)
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
    Processing::state(OrderState::Finished)
}

//
// ─── Landing the change ─────────────────────────────────────────────────────────
//

/// What a landing checks before it takes the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// Onto ground secured in advance: nothing is re-tested but what the
    /// entity itself carries and where it stands.
    Reserved,
    /// Onto ground last judged when the change was ordered: the fields and
    /// the footprint are tested again, and the landing is refused if either
    /// no longer allows the form.
    Revalidated,
    /// Back into the form the change started from, on the footprint it never
    /// left: nothing is tested.
    Return,
}

/// The landing: rewrites the entity's type in place, or returns `false` with
/// everything untouched when the change cannot happen — an unregistered
/// destination, a cell-model mover caught between cells, cargo the new form
/// cannot seat, or a destination footprint that no longer fits. Which of those
/// a landing checks before it takes the ground is its [`Landing`].
fn land(world: &mut World, entity: Entity, type_name: &str, landing: Landing) -> bool {
    let Some((type_id, to)) = destination(world, type_name) else {
        return false;
    };
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

    match landing {
        Landing::Reserved | Landing::Revalidated => {
            if let MovementModel::Cell = world.resource::<Map>().movement_model()
                && super::movement::is_mid_crossing(position)
            {
                return false;
            }
            if !cargo_fits(world, entity, type_id) {
                return false;
            }
        }
        // The form returned to stood here with this cargo before the change
        // began, and an interim form stands on the same footprint.
        Landing::Return => {}
    }

    // The footprint is anchored at its origin, so a size change recentres it:
    // growing from the same corner would shift the unit's middle sideways.
    let anchor = recentred(position, from, to);
    if !reoccupy(world, entity, position, anchor, from, to, type_id, landing) {
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
    spawn::fit_components(
        world,
        entity,
        type_id,
        FieldReach::Initial,
        StandingActs::Keep,
    );

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
/// restores exactly what came off. Only a revalidated landing tests the
/// ground; the other landings swap unconditionally.
#[allow(clippy::too_many_arguments)]
fn reoccupy(
    world: &mut World,
    entity: Entity,
    position: FixedUVec2,
    anchor: FixedUVec2,
    from: LocationDef,
    to: LocationDef,
    type_id: EntityTypeId,
    landing: Landing,
) -> bool {
    // A hidden entity holds no cells at all, so there is nothing to move.
    if world.entity(entity).contains::<HiddenComponent>() {
        return true;
    }
    // Fields judge the destination form where it will stand, like any
    // placement of that form — whatever its footprint.
    match landing {
        Landing::Revalidated => {
            let owner = entity_def::owner(world, entity);
            let def = world.resource::<ContentRegistry>().def(type_id);
            if !fields::allows_placement(world, owner, def, body::anchor(anchor)) {
                return false;
            }
        }
        Landing::Reserved | Landing::Return => {}
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
    let fits = match landing {
        Landing::Revalidated => map.can_place_entity(&placed, &to),
        Landing::Reserved | Landing::Return => true,
    };
    if !fits {
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
            from: entity_def::type_id(world, entity),
            into: type_name.to_string(),
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

    // Fields judge the destination form where it will stand.
    {
        let owner = entity_def::owner(world, entity);
        let def = world
            .resource::<ContentRegistry>()
            .entity(type_name)
            .expect("destination resolved above");
        if !fields::allows_placement(world, owner, def, body::anchor(anchor)) {
            return None;
        }
    }

    // The entity's own presence comes off before the destination is tested,
    // exactly as the landing itself will do, and goes straight back either way.
    let mut map = world.resource_mut::<Map>();
    let own = lift_standing_presence(&mut map, &standing, &from, old_class);
    let fits = map.can_place_entity(&placed, &to);
    restore_standing_presence(&mut map, &standing, &from, old_class, &own);
    if !fits {
        return None;
    }

    let cells = map.reserve_claim(to.occupation(), body::anchor(anchor), to.size());
    Some(MorphComponent {
        from: entity_def::type_id(world, entity),
        into: type_name.to_string(),
        progress: 0,
        reservation: Some(MorphReservation {
            cells,
            mask: to.occupation(),
        }),
    })
}

/// Puts an entity that ended its change early back into the form it started
/// from, when it was wearing an interim form. The return cannot be refused:
/// the interim form stands on the origin's footprint, and the origin is not
/// judged again on ground it already stood on, whatever the fields there say
/// by now.
fn return_to_origin(world: &mut World, entity: Entity, morph: &MorphComponent) {
    if entity_def::type_id(world, entity) == morph.from {
        return;
    }
    let origin = world
        .resource::<ContentRegistry>()
        .def(morph.from)
        .name
        .clone();
    let returned = land(world, entity, &origin, Landing::Return);
    debug_assert!(returned, "a return to the origin form is unconditional");
}

/// Whether the change `morph` describes has landed: the entity now is what it
/// was changing into.
fn land_succeeded(world: &World, entity: Entity, morph: &MorphComponent) -> bool {
    entity_def::of(world, entity).name == morph.into
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

/// The terms of a change under way, read from the type that declared it —
/// which the entity may no longer be, when it wears an interim form.
fn terms(world: &World, morph: &MorphComponent) -> Option<MorphTransition> {
    world
        .resource::<ContentRegistry>()
        .def(morph.from)
        .morphs
        .iter()
        .find(|transition| transition.into_type() == morph.into)
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
