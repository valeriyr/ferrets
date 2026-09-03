//! Repair order implementation.
//! Called by [`super::orders`] as part of the shared order lifecycle.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use std::collections::BTreeMap;

use bevy_ecs::{entity::Entity, world::World};
use ferrets_math::FixedU64;

use super::{
    chase::{self, Destination},
    crew,
    orders::Processing,
    work,
};
use crate::{
    components::{
        build::UnderConstructionComponent,
        energy::EnergyComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        order_queue::{CancelPolicy, OrderState},
        owner::OwnerComponent,
        repair::{RepairComponent, UnderRepairComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::SpendCause,
    map::Map,
    order::Order,
    resources::{self, PlayerResources},
    session::GameSession,
    simulation_id::SimulationId,
    spawn,
};
use ferrets_content::{
    costs::Cost,
    entity_stats::EntityStatId,
    repair::{RepairCost, RepairRate, RepairerDef},
};

/// The fractional cost carried between ticks, by resource kind.
type Carried = BTreeMap<String, FixedU64>;

/// Called once when a Repair order becomes the front `New` entry.
///
/// Inserts the driver component and returns `InProcessing`, or `Finished`
/// immediately when the entity will not mend this target — see [`accepts`].
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target_id = order
        .repair_target()
        .expect("Repair order must have a target");

    let Some(target) = world
        .resource::<EntityIndex>()
        .interactable(world, target_id)
    else {
        return OrderState::Finished;
    };
    if !accepts(world, entity, target) {
        return OrderState::Finished;
    }
    // Nothing to mend, so the walk is not worth starting.
    if remaining_damage(world, target) == FixedU64::ZERO {
        return OrderState::Finished;
    }
    if job_excludes(world, target, entity) {
        return OrderState::Finished;
    }

    crew::join::<UnderRepairComponent>(world, target, entity);
    world
        .entity_mut(entity)
        .insert(RepairComponent::new(target_id));
    OrderState::InProcessing
}

/// Called when a Repair order resumes from `Suspended` (its walk to the target
/// just finished). The driver component survives suspension; validation happens in
/// [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Repair entry that has a cancel policy.
///
/// Work stops immediately under both policies. Health already restored stays
/// restored and stays paid for; a worker standing inside its job comes back out.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    let Some(repair) = world.entity_mut(entity).take::<RepairComponent>() else {
        return OrderState::Finished;
    };
    leave_job(world, entity, repair.target);
    OrderState::Finished
}

/// Advance a Repair order by one tick.
///
/// Walk to within the mender's `repair_range` of the target (suspending on a
/// chase move), then restore one tick's worth of health and pay for it. The order
/// finishes when the pool is full, the target is gone or no longer eligible, the
/// target cannot be reached, or the worker waits out its patience unable to pay.
pub fn process(entity: Entity, _order: &Order, world: &mut World) -> Processing {
    let Some(mut repair) = world.entity_mut(entity).take::<RepairComponent>() else {
        return Processing::state(OrderState::Finished);
    };
    let target_id = repair.target;

    let Some(target) = world.resource::<EntityIndex>().alive(target_id) else {
        return finish(world, entity, target_id);
    };
    // Construction has its own order, so a site that went back to being built is no
    // longer this order's business.
    if world
        .entity(target)
        .contains::<UnderConstructionComponent>()
    {
        return finish(world, entity, target_id);
    }

    // A worker inside its job holds no cells and has nothing left to walk. One in the
    // open closes on its target every tick, since a patient can walk away from the
    // hands mending it.
    if !repair.inside_job {
        let (target_position, target_size) = entity_def::footprint(world, target);
        let projection = world.resource::<Map>().projection();
        let (chaser_position, chaser_size) = entity_def::footprint(world, entity);
        match chase::advance(
            &mut repair.last_chase,
            projection,
            chaser_position,
            chaser_size,
            target_position,
            target_size,
            work::reach(world, entity, EntityStatId::REPAIR_RANGE),
        ) {
            Destination::OutOfReach => return finish(world, entity, target_id),
            Destination::Walk(move_order) => {
                world.entity_mut(entity).insert(repair);
                return Processing::suspend(move_order);
            }
            Destination::Arrived => {}
        }
        chase::face(world, entity, target_position, target_size);

        // Stepping inside the job frees the cell the worker was holding. Done on
        // arrival, so it happens exactly once — and only for a worker whose presence
        // says so, never because something else left it off the map.
        if repairer_of(world, entity).presence().is_hidden() {
            spawn::hide_entity(world, entity);
            repair.inside_job = true;
        }
    }

    let max_health = effective(world, target, EntityStatId::MAX_HEALTH);
    let restored = restorable(world, entity, target, max_health);
    if restored == FixedU64::ZERO {
        return finish(world, entity, target_id);
    }

    let Some(carried) = charge(world, entity, target, restored, max_health, &repair.owed) else {
        // Broke: the job is held without progressing, and abandoned once patience
        // runs out. Nothing is charged and nothing is mended this tick.
        repair.stalled += 1;
        let patience = repairer_of(world, entity).patience();
        if patience.is_some_and(|limit| repair.stalled > limit) {
            return finish(world, entity, target_id);
        }
        world.entity_mut(entity).insert(repair);
        return Processing::state(OrderState::InProcessing);
    };

    repair.stalled = 0;
    repair.owed = carried;
    world
        .entity_mut(target)
        .get_mut::<HealthComponent>()
        .expect("a mendable target is damageable")
        .heal(restored, max_health);
    world.entity_mut(entity).insert(repair);
    Processing::state(OrderState::InProcessing)
}

/// Whether sending `entity` to `target` should be read as an offer to mend it —
/// [`accepts`] plus something actually being wrong with the target.
pub(super) fn would_repair(world: &World, entity: Entity, target: Entity) -> bool {
    accepts(world, entity, target) && remaining_damage(world, target) > FixedU64::ZERO
}

/// Whether `entity` will mend `target`: it has the capability, the target is
/// repairable and carries a tag it mends, the two are on the same side, and
/// self-repair is either allowed or not being asked for.
fn accepts(world: &World, entity: Entity, target: Entity) -> bool {
    let Some(repairer) = entity_def::of(world, entity).repairer.as_ref() else {
        return false;
    };
    if entity == target && !repairer.self_repair() {
        return false;
    }
    if world
        .entity(target)
        .contains::<UnderConstructionComponent>()
    {
        return false;
    }
    let target_def = entity_def::of(world, target);
    if !target_def.has_health() || !repairer.mends(&target_def.tags) {
        return false;
    }
    // Only a production-paced mender needs the target to be something production
    // knows a duration for.
    if repairer.rate() == RepairRate::Production && !target_def.is_production_repairable() {
        return false;
    }
    // Mending an enemy is never the intent, and a neutral belongs to nobody.
    matches!(
        (
            world.entity(entity).get::<OwnerComponent>(),
            world.entity(target).get::<OwnerComponent>(),
        ),
        (Some(worker), Some(subject))
            if world
                .resource::<GameSession>()
                .are_allied(worker.player(), subject.player())
    )
}

/// Whether `entity` is shut out of mending `target` by the crew already on it.
fn job_excludes(world: &World, target: Entity, entity: Entity) -> bool {
    crew::excludes::<UnderRepairComponent>(world, target, entity, shares_jobs)
}

/// Drops out of the crew on `target`, taking [`UnderRepairComponent`] with it as the
/// last worker to leave.
///
/// A target already on its way off the map keeps nothing to leave.
fn leave_crew(world: &mut World, target: SimulationId, entity: Entity) {
    let Some(target) = world.resource::<EntityIndex>().alive(target) else {
        return;
    };
    crew::leave_and_unmark::<UnderRepairComponent>(world, target, entity);
}

/// Whether an entity's repair capability lets several workers share one job.
fn shares_jobs(world: &World, entity: Entity) -> bool {
    entity_def::of(world, entity)
        .repairer
        .as_ref()
        .is_some_and(|repairer| repairer.presence().stacks())
}

/// The health one tick of `entity`'s work restores on `target`, capped by what the
/// pool is missing.
fn restorable(world: &World, entity: Entity, target: Entity, max_health: FixedU64) -> FixedU64 {
    // The speed stat scales whichever pace content chose, so an upgrade reads the
    // same way for a builder mending a wall and a medic patching up infantry.
    let speed = effective(world, entity, EntityStatId::REPAIR_SPEED);
    let amount = match repairer_of(world, entity).rate() {
        RepairRate::PerTick(health) => health.saturating_mul(speed),
        RepairRate::Production => {
            let target_def = entity_def::of(world, target);
            let production = FixedU64::from_num(target_def.production_time().unwrap_or_default());
            let ratio = target_def.repair_ratio.unwrap_or(FixedU64::ONE);

            // Ticks one worker at a speed of `1` needs for a full pool. A production
            // time and ratio small enough to vanish in fixed point would divide by
            // zero, so the work lands in a single tick instead.
            let full_repair = production.saturating_mul(ratio);
            if full_repair == FixedU64::ZERO {
                max_health
            } else {
                max_health.saturating_mul(speed) / full_repair
            }
        }
    };
    amount.min(remaining_damage(world, target))
}

/// The health `target` is missing from its effective pool.
fn remaining_damage(world: &World, target: Entity) -> FixedU64 {
    let max_health = effective(world, target, EntityStatId::MAX_HEALTH);
    let current = world
        .entity(target)
        .get::<HealthComponent>()
        .expect("a mendable target is damageable")
        .current();
    max_health.saturating_sub(current)
}

/// Pays for `restored` health of work, returning the fractional remainder to carry
/// into the next tick, or `None` when the owner cannot afford this tick's bill.
///
/// A pro-rata charge accumulates rather than rounding per tick — see [`pro_rata`]
/// for what that does and does not guarantee. A flat charge is billed in full every
/// tick a worker works, so a crew pays once per worker.
fn charge(
    world: &mut World,
    entity: Entity,
    target: Entity,
    restored: FixedU64,
    max_health: FixedU64,
    owed: &Carried,
) -> Option<Carried> {
    let (due, carried) = match repairer_of(world, entity).cost() {
        RepairCost::Free => (Cost::new(), owed.clone()),
        RepairCost::PerTick(cost) => (cost.clone(), owed.clone()),
        // Paid out of the worker rather than the treasury, and in one piece: energy
        // is already fractional, so there is nothing to carry between ticks.
        RepairCost::Energy(per_health) => {
            let spend = per_health.saturating_mul(restored);
            let paid = world
                .entity_mut(entity)
                .get_mut::<EnergyComponent>()
                .expect("a mender paying with energy has a pool")
                .spend(spend);
            return paid.then(|| owed.clone());
        }
        // `max_health` cannot be zero — the stat floors at one — so the division
        // inside is safe.
        RepairCost::ProRata => {
            let factor = effective(world, entity, EntityStatId::REPAIR_COST_FACTOR);
            let target_cost = entity_def::of(world, target).cost.clone();
            pro_rata(&target_cost, restored, max_health, factor, owed)
        }
    };

    let player = world
        .entity(entity)
        .get::<OwnerComponent>()
        .expect("repair only starts between owned entities — see accepts")
        .player();
    if !world.resource::<PlayerResources>().can_afford(player, &due) {
        return None;
    }
    resources::charge(
        world,
        player,
        due,
        SpendCause::Repair {
            target: entity_def::simulation_id(world, target),
        },
    );
    Some(carried)
}

/// Splits a pro-rata bill into what is spent now and what is carried on, given the
/// remainder left over from earlier ticks.
///
/// The share of the pool is folded in as `restored / max_health` with the division
/// taken last, so a rate that divides the pool evenly bills whole amounts instead of
/// shedding a fraction on every tick. A job can still end owing less than one unit
/// of a resource, which is never charged — the same on every peer.
fn pro_rata(
    target_cost: &Cost,
    restored: FixedU64,
    max_health: FixedU64,
    factor: FixedU64,
    owed: &Carried,
) -> (Cost, Carried) {
    let mut due = Cost::new();
    let mut carried = owed.clone();
    for (kind, &amount) in target_cost {
        let share = FixedU64::from_num(amount)
            .saturating_mul(factor)
            .saturating_mul(restored)
            / max_health;
        let total = carried.get(kind).copied().unwrap_or(FixedU64::ZERO) + share;
        let whole = total.floor();
        if whole > FixedU64::ZERO {
            due.insert(kind.clone(), whole.to_num::<u32>());
        }
        carried.insert(kind.clone(), total - whole);
    }
    (due, carried)
}

/// Ends the order, bringing a worker that stood inside its job back out.
fn finish(world: &mut World, entity: Entity, target: SimulationId) -> Processing {
    leave_job(world, entity, target);
    Processing::state(OrderState::Finished)
}

/// Leaves the job: the worker drops out of the target's crew, and one that stood
/// inside comes back on the map beside it — or beside itself when the target is
/// already gone. A worker that mends from the open is left exactly where it stands.
fn leave_job(world: &mut World, entity: Entity, target: SimulationId) {
    leave_crew(world, target, entity);

    let (around, around_size) = match world.resource::<EntityIndex>().alive(target) {
        Some(target) => {
            let (position, size) = entity_def::footprint(world, target);
            (CellPos::from(position), size)
        }
        None => (
            CellPos::from(entity_def::position(world, entity)),
            CellSize::ONE,
        ),
    };
    work::leave(world, entity, around, around_size);
}

/// The repair capability of an entity partway through a repair order.
fn repairer_of(world: &World, entity: Entity) -> &RepairerDef {
    entity_def::of(world, entity)
        .repairer
        .as_ref()
        .expect("a repair order only starts on an entity that can repair")
}

/// One effective stat of an entity.
///
/// Every stat read through this is one registration demands of the capability that
/// reads it, and a repair order only starts on an entity that has that capability.
fn effective(world: &World, entity: Entity, stat: EntityStatId) -> FixedU64 {
    world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("menders and their targets have a stat store")
        .effective(stat)
        .unwrap()
}
