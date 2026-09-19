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
//! [`MorphTransition`] the entity's own type declares. A transition may name
//! an interim form, worn from the start of the change until it lands, and
//! returned from when the change ends early.
//!
//! The order runs in four steps. A start is **judged** in full — every check,
//! and every cell the start will need — before anything changes, and then
//! **begun** in steps none of which can fail. The change **lands** when its
//! progress runs out, on the transition's terms for a landing that finds its
//! ground gone, and **ends early** on its terms for a cancel. A change that
//! cannot happen finishes silently, like any other order that finds its work
//! impossible.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeId,
    location::LocationDef,
    morph::{MorphCancel, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition},
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use super::orders::{self, Processing, Refusal};
use crate::{
    berths, brood,
    components::{
        attached::AttachedComponent,
        dying::DyingComponent,
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        location::LocationComponent,
        morph::{MorphComponent, MorphReservation},
        movement::MoveComponent,
        order_queue::{CancelPolicy, OrderQueueComponent, OrderState},
        transport::TransporterComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent, SpendCause},
    fields,
    game_loop::cast_cost,
    map::{Map, OccupancyClass},
    movement_model::{self, MovementModel},
    order::Order,
    rally, requirements,
    session::player_id::PlayerId,
    simulation_id::SimulationId,
    spawn::{self, FieldReach, StandingActs},
    supply,
};

//
// ─── The order ──────────────────────────────────────────────────────────────
//

/// Whether `entity` may start this Morph: see [`judge`]. Requirements and the
/// cost are judged when the order starts, by the caller that pays.
pub fn can_start(world: &World, entity: Entity, order: &Order) -> Result<(), Refusal> {
    let Order::Morph { type_name } = order else {
        unreachable!("can_start called with a non-Morph order");
    };
    judge(world, entity, type_name).map(|_| ())
}

/// Called once when a Morph order becomes the front `New` entry.
///
/// Judges the start again — the order may have waited in the queue since it
/// was judged at issue — along with the requirements and the cost, and only
/// then begins the change. A refused start finishes the order with nothing
/// touched.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let Order::Morph { type_name } = order else {
        unreachable!("prepare called with a non-Morph order");
    };
    let Ok(start) = judge(world, entity, type_name) else {
        return OrderState::Finished;
    };
    let Some(player) = entity_def::owner(world, entity) else {
        return OrderState::Finished;
    };
    if !requirements::met(world, player, Some(entity), start.transition.requires())
        || !cast_cost::can_pay(world, entity, player, start.transition.costs())
    {
        return OrderState::Finished;
    }
    begin(world, entity, player, type_name, start)
}

/// Advance a Morph order by one tick.
///
/// The unit does nothing else while the progress runs; when it runs out the
/// change lands on the transition's own terms.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let mut morph = world
        .entity_mut(entity)
        .take::<MorphComponent>()
        .expect("a Morph order under way carries its change");
    let transition = entity_def::morph_terms(world, &morph)
        .expect("a change under way is one the form it came from declares");
    let time = entity_def::quantity(world, entity, transition.time());

    morph.progress += 1;
    if morph.progress < time {
        world.entity_mut(entity).insert(morph);
        return Processing::state(OrderState::InProcessing);
    }
    match finish(world, entity, morph, &transition) {
        Finish::Landed | Finish::Standing => Processing::state(OrderState::Finished),
        Finish::Dying => Processing::finished_dying(),
    }
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
) -> Processing {
    let Order::Morph { .. } = order else {
        unreachable!("cancel_processing called with a non-Morph order");
    };
    // A queued entry was never prepared: nothing was paid or reserved for it,
    // and the change on the entity, if any, is the one under way in front.
    match entry_state {
        OrderState::New => return Processing::state(OrderState::Finished),
        OrderState::InProcessing | OrderState::Suspended => {}
        OrderState::Finished => unreachable!("Finished entries never stay in the queue"),
    }

    let morph = world
        .entity(entity)
        .get::<MorphComponent>()
        .expect("a Morph order under way carries its change");
    let transition = entity_def::morph_terms(world, morph)
        .expect("a change under way is one the form it came from declares");
    let payment = match (transition.cancel(), policy) {
        // A committed window refuses the player's cancel only once it is
        // actually open: an entry still queued has taken nothing and
        // promised nothing, and drops like any other.
        (MorphCancel::Committed, CancelPolicy::Soft) => {
            if entry_state == OrderState::InProcessing {
                return Processing::state(OrderState::InProcessing);
            }
            Payment::Kept
        }
        (MorphCancel::Committed, CancelPolicy::Force) => Payment::Kept,
        (MorphCancel::Forfeit, _) => Payment::Kept,
        (MorphCancel::Refundable, _) => Payment::Returned,
    };

    let morph = world
        .entity_mut(entity)
        .take::<MorphComponent>()
        .expect("a Morph order under way carries its change");
    release_reservation(world, &morph);
    // A dying entity keeps the form its death was announced in, and so does
    // one about to die on its transition's terms.
    if world.entity(entity).contains::<DyingComponent>() {
        return Processing::state(OrderState::Finished);
    }
    match abandon(world, entity, &morph, &transition, payment) {
        EarlyEnd::Standing => Processing::state(OrderState::Finished),
        EarlyEnd::Dying => Processing::finished_dying(),
    }
}

/// Whether a Morph can stand through a soft cancel as a kind: no — a change
/// still queued drops like any other order, and one under way answers by its
/// own cancel terms in [`cancel_processing`].
pub fn survives_soft_cancel() -> bool {
    false
}

/// Calls off the change of form `entity` is under, for `player`.
///
/// The change answers on its own terms, as it does for any other cancel: a
/// refundable one gives the price back and returns the entity to what it was,
/// a forfeit one keeps the price, and a committed one holds until its window
/// closes. Nothing happens for an entity that is not the player's or is not
/// changing at all.
pub fn cancel_change(world: &mut World, player: PlayerId, entity: SimulationId) {
    let Some(entity) = world.resource::<EntityIndex>().interactable(world, entity) else {
        return;
    };
    if entity_def::owner(world, entity) != Some(player) {
        return;
    }
    let mut entity_mut = world.entity_mut(entity);
    let Some(mut queue) = entity_mut.get_mut::<OrderQueueComponent>() else {
        return;
    };
    let Some(front) = queue.front_mut() else {
        return;
    };
    // Only the change itself is called off: whatever else the entity has
    // queued is none of this command's business.
    match front.order {
        Order::Morph { .. } => front.cancel = Some(CancelPolicy::Soft),
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::AttackMove { .. }
        | Order::Patrol { .. }
        | Order::Guard { .. }
        | Order::Follow { .. }
        | Order::Board { .. }
        | Order::Load { .. }
        | Order::Unload { .. }
        | Order::Harvest { .. }
        | Order::Build { .. }
        | Order::Repair { .. }
        | Order::Train
        | Order::Research { .. }
        | Order::Cast { .. }
        | Order::Die => {}
    }
}

/// Whether `player` meets the requirements of `entity`'s transition into
/// `type_name`. A transition the type does not declare has none to meet.
pub fn requirements_met(world: &World, player: PlayerId, entity: Entity, type_name: &str) -> bool {
    transition_into(world, entity, type_name).is_none_or(|transition| {
        requirements::met(world, player, Some(entity), transition.requires())
    })
}

//
// ─── Judging a start ────────────────────────────────────────────────────────
//

/// Everything a start needs, settled before anything changes and applied as
/// found.
struct Start {
    /// The transition, as the entity's type declares it.
    transition: MorphTransition,
    /// The cell an attached entity steps onto before it changes, one that
    /// takes the form it wears next; `None` for an entity already standing.
    step_out: Option<CellPos>,
    /// The anchor and location a reserving change claims, judged from where
    /// the entity will stand; `None` for a placement that claims nothing.
    reservation: Option<(FixedUVec2, LocationDef)>,
}

/// Judges a change of `entity` into `type_name` without touching anything,
/// and settles what beginning it needs. Refused when the type declares no
/// such transition or the destination is nothing to stand as, when the
/// destination cannot seat what is aboard, when the player's supply has no
/// room for what the new form costs over the old, when the queue holds work a
/// soft flush would leave or others sit in the entity's berths, when the
/// entity does not operate, and when the ground refuses: no cell for an
/// attached entity to step onto, none under the interim form where it will
/// stand, or none under the footprint a reserving change would claim.
fn judge(world: &World, entity: Entity, type_name: &str) -> Result<Start, Refusal> {
    let Some(transition) = transition_into(world, entity, type_name) else {
        return Err(Refusal::Incapable);
    };
    let Some((type_id, to)) = destination(world, type_name) else {
        return Err(Refusal::Incapable);
    };
    let Some(from) = entity_def::of(world, entity).location else {
        return Err(Refusal::Incapable);
    };
    if !cargo_fits(world, entity, type_id) {
        return Err(Refusal::TargetUnfit);
    }
    if let Some(player) = entity_def::owner(world, entity)
        && !supply::allows_change(
            world,
            player,
            entity,
            world.resource::<ContentRegistry>().def(type_id),
        )
    {
        return Err(Refusal::NoSupply);
    }
    // Work a soft flush would leave in the queue — paid production, a change
    // already under way — is not something a change of form may run off with,
    // and neither are the workers seated in its berths: the player finishes or
    // cancels that work first. Its own broodlings are not workers, and are
    // aligned with the new form when the change lands. What a docked annex is
    // doing is the annex's own business: it keeps its queue, and its terms for
    // standing alone say whether the work carries on or waits for the next
    // primary.
    if orders::resists_soft_flush(world, entity)
        || berths::seated_by_others(world, entity, brood::broodlings(world, entity))
    {
        return Err(Refusal::Busy);
    }
    orders::requires_operating(world, entity)?;

    // An attached entity holds no cells: before it changes it steps onto
    // ground that takes the form it wears next, the nearest to its berth.
    let step_out = if world.entity(entity).contains::<AttachedComponent>() {
        let Some(worn) = worn_next(world, &transition) else {
            return Err(Refusal::Incapable);
        };
        let berth = body::anchor(entity_def::position(world, entity));
        match spawn::cell_back_near(world, entity, berth, from.size(), &worn) {
            Some(cell) => Some(cell),
            None => return Err(Refusal::NoRoom),
        }
    } else {
        None
    };
    // Where the change begins, and what the entity holds there: a body that
    // steps out holds nothing yet, one off the grid nothing at all, one
    // standing its own footprint — which the forms it takes may overlap.
    let standing = step_out.map_or_else(|| entity_def::position(world, entity), FixedUVec2::from);
    let on_grid = step_out.is_some() || entity_def::stands_on_grid(world, entity);
    let own = world
        .entity(entity)
        .get::<LocationComponent>()
        .copied()
        .filter(|_| step_out.is_none() && on_grid);
    let own_class = OccupancyClass::of(entity_def::of(world, entity));
    let allowed = |worn_id: EntityTypeId, anchor: FixedUVec2| {
        let def = world.resource::<ContentRegistry>().def(worn_id);
        fields::allows_placement(
            world,
            entity_def::owner(world, entity),
            def,
            body::anchor(anchor),
        )
    };
    let takes = |worn_id: EntityTypeId, worn: LocationDef, anchor: FixedUVec2| {
        let placed = LocationComponent::new(anchor, spawn::DEFAULT_FACING);
        allowed(worn_id, anchor)
            && world.resource::<Map>().can_place_entity_over_own(
                &placed,
                &worn,
                own.as_ref().map(|location| (location, &from, own_class)),
            )
    };

    let reservation = match transition.placement() {
        MorphPlacement::Reserve if on_grid => {
            let anchor = settled(world, type_id, standing, from, to);
            if !takes(type_id, to, anchor) {
                return Err(Refusal::NoRoom);
            }
            Some((anchor, to))
        }
        // Off the grid there is nothing to secure, and the landing asks
        // nothing of the ground either.
        MorphPlacement::Reserve | MorphPlacement::Revalidate | MorphPlacement::Nearby => None,
    };
    // An interim form must seat what is aboard like the destination, and is
    // judged by the fields where it will stand; one that holds the ground as
    // the origin does takes the origin's own footprint without asking the
    // grid, one that holds it differently asks.
    if let Some(via) = transition.via_type() {
        let Some((via_id, worn)) = destination(world, via) else {
            return Err(Refusal::Incapable);
        };
        if !cargo_fits(world, entity, via_id) {
            return Err(Refusal::TargetUnfit);
        }
        if on_grid {
            let anchor = settled(world, via_id, standing, from, worn);
            let stands = if worn.solidity() == from.solidity() {
                allowed(via_id, anchor)
            } else {
                takes(via_id, worn, anchor)
            };
            if !stands {
                return Err(Refusal::NoRoom);
            }
        }
    }
    Ok(Start {
        transition,
        step_out,
        reservation,
    })
}

//
// ─── Beginning the change ───────────────────────────────────────────────────
//

/// Begins the change `start` settled, in steps none of which can fail: an
/// attached entity steps out onto its cell, a reserving change claims its
/// footprint, the cost is drawn, the change is marked under way, and the
/// interim form is put on. The landing is [`process`]'s, a change with no
/// time to run landing on its first process — the same tick it begins.
fn begin(
    world: &mut World,
    entity: Entity,
    player: PlayerId,
    type_name: &str,
    start: Start,
) -> OrderState {
    let Start {
        transition,
        step_out,
        reservation,
    } = start;
    if let Some(cell) = step_out {
        spawn::step_out(world, entity, cell);
        brood::uncount(world, entity);
    }
    debug_assert!(
        !matches!(
            world.resource::<Map>().movement_model(),
            MovementModel::Cell
        ) || !movement_model::is_mid_crossing(entity_def::position(world, entity)),
        "a cell-model mover finishes its crossing before another order starts"
    );
    let reservation = reservation.map(|(anchor, to)| MorphReservation {
        cells: world.resource_mut::<Map>().reserve_claim(
            to.occupation(),
            body::anchor(anchor),
            to.size(),
        ),
        mask: to.occupation(),
    });
    cast_cost::pay(
        world,
        entity,
        player,
        transition.costs(),
        SpendCause::Morph {
            entity: entity_def::simulation_id(world, entity),
        },
    );
    let morph = MorphComponent {
        from: entity_def::type_id(world, entity),
        into: type_name.to_string(),
        progress: 0,
        reservation,
    };
    // The change is under way from here, so what the interim form's fitting
    // keeps for the duration — a brood being counted — sees it so.
    world.entity_mut(entity).insert(morph.clone());
    if let Some(via) = transition.via_type() {
        let worn = land(
            world,
            entity,
            via,
            Landing::Reserved,
            morph.from,
            interim_of(world, &transition),
        );
        debug_assert!(
            worn,
            "the interim form was judged to stand where the change begins"
        );
    }
    OrderState::InProcessing
}

//
// ─── Landing the change ─────────────────────────────────────────────────────
//

/// What a change came to when its progress ran out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// Standing in the new form.
    Landed,
    /// Standing in the form the change started from: the landing was refused.
    Standing,
    /// Dying, as the transition said it would when the landing was refused.
    Dying,
}

/// Ends the change `morph` whose progress has run out: a reserving change
/// lets go of its reservation and takes the ground it held without asking
/// again, a revalidating one re-checks the destination, a nearby one searches
/// for it. A refused landing is a fizzle: the entity meets the transition's
/// terms for an early end — back in the form it started from, or dead in the
/// one it wore — and a refundable transition's payment goes back the same way
/// a cancel returns it.
fn finish(
    world: &mut World,
    entity: Entity,
    morph: MorphComponent,
    transition: &MorphTransition,
) -> Finish {
    // Between the release and the landing nothing else runs, so the ground a
    // reservation held passes to the new footprint atomically.
    release_reservation(world, &morph);
    let landing = match transition.placement() {
        MorphPlacement::Reserve => Landing::Reserved,
        MorphPlacement::Revalidate => Landing::Revalidated,
        MorphPlacement::Nearby => Landing::Nearby,
    };
    // The rally point a producing form carries goes with the landing refit;
    // read first, it sends the unit made on. A change that makes no unit
    // sends nobody anywhere.
    let own = match transition.reason() {
        MorphReason::Production => rally::target_of(world, entity),
        MorphReason::Change => None,
    };
    if land(
        world,
        entity,
        &morph.into,
        landing,
        morph.from,
        interim_of(world, transition),
    ) {
        brood::hatch(world, entity, own);
        brood::open(world, entity);
        return Finish::Landed;
    }
    // The change fizzled: a refundable transition's payment goes back the
    // same way a cancel returns it.
    let payment = match transition.cancel() {
        MorphCancel::Refundable => Payment::Returned,
        MorphCancel::Committed | MorphCancel::Forfeit => Payment::Kept,
    };
    match abandon(world, entity, &morph, transition, payment) {
        EarlyEnd::Standing => Finish::Standing,
        EarlyEnd::Dying => Finish::Dying,
    }
}

/// What a landing checks before it takes the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// Onto ground secured in advance, or judged as the change began: nothing
    /// is re-tested but what the entity itself carries and where it stands.
    Reserved,
    /// Onto ground last judged when the change was ordered: the fields and
    /// the footprint are tested again, and the landing is refused if either
    /// no longer allows the form.
    Revalidated,
    /// Onto the nearest ground that takes the form, searched from where the
    /// entity stands; refused only when nothing within the search radius
    /// does.
    Nearby,
    /// Back into the form the change started from, on the footprint it never
    /// left: nothing is tested.
    Return,
}

/// The landing: rewrites the entity's type in place, or returns `false` with
/// everything untouched when the change cannot happen — an unregistered
/// destination, a cell-model mover caught between cells, cargo the new form
/// cannot seat, or a destination footprint that no longer fits. Which of those
/// a landing checks before it takes the ground is its [`Landing`].
fn land(
    world: &mut World,
    entity: Entity,
    type_name: &str,
    landing: Landing,
    origin: EntityTypeId,
    interim: Option<EntityTypeId>,
) -> bool {
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
        Landing::Reserved | Landing::Revalidated | Landing::Nearby => {
            if let MovementModel::Cell = world.resource::<Map>().movement_model()
                && movement_model::is_mid_crossing(position)
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
    // growing from the same corner would shift the unit's middle sideways. A
    // form that stands still then settles onto the cells it holds — or, on a
    // nearby landing, onto the nearest cells that take it.
    let anchor = settled(world, type_id, position, from, to);
    let Some(anchor) = reoccupy(world, entity, position, anchor, from, to, type_id, landing) else {
        return false;
    };
    // The brood seated on the old form, to be aligned with the new — its own
    // copy, since the world changes under it first.
    let broodlings = brood::broodlings(world, entity).to_vec();

    // Identity stays; the type is rewritten under it, and the anchor follows.
    let base_stats = world
        .resource::<ContentRegistry>()
        .def(type_id)
        .base_stats
        .clone();
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
        if interim == Some(type_id) {
            spawn::Wearing::Interim
        } else {
            spawn::Wearing::Own
        },
    );
    spawn::align_broodlings(world, entity, &broodlings, origin);

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
        from: origin,
        to: type_id,
        interim,
    };
    world.resource_mut::<EventRecord>().emit(announced);

    true
}

/// Moves the entity's presence on the grid from the old footprint to the new
/// one, returning the anchor the new footprint took, or `None` with the old
/// footprint untouched when the new one does not fit.
///
/// The decision comes before anything moves: a revalidated landing asks the
/// fields and the grid about its anchor, looking past the footprint the entity
/// holds now, since a form's own presence never stands in its way; a nearby
/// landing searches out from the footprint for ground that takes the form —
/// the search has no way to look past the entity's own footprint, so that
/// comes off for the search and goes straight back; the other landings swap
/// unconditionally. Only then does the old footprint come off and the new one
/// go on. A static footprint is displaced and placed; a mover's claim is
/// lifted from where the claim plane actually holds it — the rounded anchor
/// the continuous rebuild stamps, which is the settled cell itself under the
/// cell model — because displacing a claim is a no-op under the continuous
/// model (its clears belong to the rebuild alone) and a put-back would mint
/// bits the plane never held.
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
) -> Option<FixedUVec2> {
    // An entity off the grid holds no cells at all, so there is nothing to move.
    if !entity_def::stands_on_grid(world, entity) {
        return Some(anchor);
    }
    let old_class = OccupancyClass::of(entity_def::of(world, entity));
    let new_class = OccupancyClass::of(world.resource::<ContentRegistry>().def(type_id));
    // Unchanged presence — the same cells on the same plane, the same way —
    // has nothing to swap; a nearby landing may still move.
    let unchanged = from.occupation() == to.occupation()
        && from.size() == to.size()
        && old_class == new_class
        && from.solidity() == to.solidity()
        && landing != Landing::Nearby;

    let facing = world
        .entity(entity)
        .get::<LocationComponent>()
        .map(|location| location.facing)
        .unwrap_or(spawn::DEFAULT_FACING);
    let standing = LocationComponent::new(position, facing);
    let owner = entity_def::owner(world, entity);
    let allowed = |world: &World, anchor: FixedUVec2| {
        let def = world.resource::<ContentRegistry>().def(type_id);
        fields::allows_placement(world, owner, def, body::anchor(anchor))
    };

    let anchor = match landing {
        Landing::Revalidated => {
            let placed = LocationComponent::new(anchor, facing);
            let fits = allowed(world, anchor)
                && (unchanged
                    || world.resource::<Map>().can_place_entity_over_own(
                        &placed,
                        &to,
                        Some((&standing, &from, old_class)),
                    ));
            if !fits {
                return None;
            }
            anchor
        }
        Landing::Nearby => {
            let mut map = world.resource_mut::<Map>();
            let own = lift_standing_presence(&mut map, &standing, &from, old_class);
            let around = CellRect::new(body::anchor(position), from.size());
            let found = map
                .find_placement_near(around.origin, around.size, &to)
                .map(FixedUVec2::from);
            restore_standing_presence(&mut map, &standing, &from, old_class, &own);
            let anchor = found?;
            if !allowed(world, anchor) {
                return None;
            }
            anchor
        }
        Landing::Reserved | Landing::Return => anchor,
    };
    if unchanged {
        return Some(anchor);
    }

    let mut map = world.resource_mut::<Map>();
    lift_standing_presence(&mut map, &standing, &from, old_class);
    map.place_entity(&LocationComponent::new(anchor, facing), &to, new_class);
    Some(anchor)
}

//
// ─── Ending early ───────────────────────────────────────────────────────────
//

/// What becomes of a change's payment when the change is abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Payment {
    /// The payment is lost with the change.
    Kept,
    /// The payment goes back to the player.
    Returned,
}

/// Abandons the change `morph` before it lands, on the transition's terms: a
/// reverting one puts the entity back in the form it started from, the
/// payment is returned when `payment` says so, and the entity meets the
/// transition's terms for an early end.
fn abandon(
    world: &mut World,
    entity: Entity,
    morph: &MorphComponent,
    transition: &MorphTransition,
    payment: Payment,
) -> EarlyEnd {
    match transition.interrupted() {
        MorphInterrupted::Reverts => return_to_origin(world, entity, morph),
        MorphInterrupted::Dies => {}
    }
    if let Payment::Returned = payment
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
    ended_early(world, entity, transition.interrupted(), morph.from)
}

/// What an entity is left as once its change ended early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EarlyEnd {
    /// Standing in the form the change started from.
    Standing,
    /// Dying, as the transition said it would.
    Dying,
}

/// Ends a change early on the transition's `interrupted` terms, once the payment
/// is settled and any interim form taken off: a returning bred entity takes a
/// seat again; a perishing one dies in the form it wears, its brood — if it
/// breeds — settled on the terms of `origin`, the form the change was declared
/// on, which the form it dies in may not declare.
fn ended_early(
    world: &mut World,
    entity: Entity,
    interrupted: MorphInterrupted,
    origin: EntityTypeId,
) -> EarlyEnd {
    match interrupted {
        MorphInterrupted::Reverts => {
            spawn::reseat_broodling(world, entity);
            if world.entity(entity).contains::<DyingComponent>() {
                EarlyEnd::Dying
            } else {
                EarlyEnd::Standing
            }
        }
        MorphInterrupted::Dies => {
            spawn::settle_broodlings(world, entity, origin);
            spawn::despawn_entity(world, entity, DeathCause::Cancelled);
            EarlyEnd::Dying
        }
    }
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
    let interim =
        entity_def::morph_terms(world, morph).and_then(|transition| interim_of(world, &transition));
    let returned = land(world, entity, &origin, Landing::Return, morph.from, interim);
    debug_assert!(returned, "a return to the origin form is unconditional");
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
// ─── The transition's terms and the ground ──────────────────────────────────
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

/// The destination type's handle and location, or `None` when it is not
/// something an entity could stand as.
fn destination(world: &World, type_name: &str) -> Option<(EntityTypeId, LocationDef)> {
    let registry = world.resource::<ContentRegistry>();
    let type_id = registry.type_id(type_name)?;
    Some((type_id, registry.def(type_id).location?))
}

/// The location of the form `entity` wears next under `transition`: the
/// interim form's when there is one, the destination's otherwise. `None` when
/// that form is not something an entity could stand as.
fn worn_next(world: &World, transition: &MorphTransition) -> Option<LocationDef> {
    let next = transition.via_type().unwrap_or(transition.into_type());
    destination(world, next).map(|(_, location)| location)
}

/// The interim form `transition` passes through, as a handle, if it has one.
fn interim_of(world: &World, transition: &MorphTransition) -> Option<EntityTypeId> {
    let via = transition.via_type()?;
    world.resource::<ContentRegistry>().type_id(via)
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
        .base_stat_as_u32(EntityStatId::CARGO_CAPACITY)
        .unwrap_or(0);

    let mut slots = 0;
    for passenger in aboard {
        let passenger_def = entity_def::of(world, passenger);
        // Admission is by type name or tag, exactly as boarding checks it: a form
        // that would not have let this passenger in cannot inherit it either.
        if !transporter.carries().admits(passenger_def) {
            return false;
        }
        slots += entity_def::effective_stat_u32(world, passenger, EntityStatId::CARGO_SIZE);
    }
    capacity >= slots
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

/// Where a form settles: the [`recentred`] anchor, on the lattice when the
/// destination stands still or the cell model keeps every body on it.
fn settled(
    world: &World,
    type_id: EntityTypeId,
    position: FixedUVec2,
    from: LocationDef,
    to: LocationDef,
) -> FixedUVec2 {
    let anchor = recentred(position, from, to);
    let class = OccupancyClass::of(world.resource::<ContentRegistry>().def(type_id));
    match (class, world.resource::<Map>().movement_model()) {
        // A standing footprint holds the cells its anchor rounds to, so a form
        // left between them would be drawn — and would reach — off its own
        // cells; a cell-model mover left there would read as mid-crossing.
        (OccupancyClass::Static, _) | (OccupancyClass::Claim, MovementModel::Cell) => {
            FixedUVec2::from(body::anchor(anchor))
        }
        (OccupancyClass::Claim, MovementModel::Continuous) => anchor,
    }
}

/// How full a pool is, as a fraction of its maximum, or `None` when there is no
/// pool to carry over.
fn filled_fraction(current: Option<FixedU64>, maximum: Option<FixedU64>) -> Option<FixedU64> {
    let (current, maximum) = current.zip(maximum)?;
    (maximum > FixedU64::ZERO).then(|| current / maximum)
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
