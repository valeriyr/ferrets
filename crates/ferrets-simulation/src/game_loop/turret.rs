//! The guns a body carries that point where they like.
//!
//! A turret is not worked by the order lifecycle, because the body it sits on may
//! be walking somewhere on business that knows nothing about the fight. It runs
//! from here instead, right after the orders have moved everything, so a shot
//! leaves from where its body now stands.
//!
//! What a gun works is what its body was ordered onto — an order is given to the
//! entity, so every gun that can reach what it named takes it — and, failing that,
//! whatever it finds for itself.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{
    registry::ContentRegistry,
    targeting,
    turret::{TurretFire, TurretId, TurretStats, WeaponConduct},
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::layer_mask::LayerMask;

use super::{acquire, attack, chase, turn};
use crate::{
    components::{
        hidden::HiddenComponent,
        movement::MoveComponent,
        order_queue::{OrderQueueComponent, OrderState},
        stance::{Stance, StanceComponent},
        turret::{TurretState, TurretsComponent},
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    impacts::FiredFrom,
    order::{AttackTarget, Order},
    session::GameSession,
    simulation_id::SimulationId,
};

/// Advances every mounted turret's fight by one tick.
///
/// Bodies are visited in id order and their guns in mounted order, so every peer
/// fires the same shots in the same sequence.
pub fn process_turrets(world: &mut World) {
    for (id, entity) in world.resource::<EntityIndex>().alive_entries() {
        if !world.entity(entity).contains::<TurretsComponent>() {
            continue;
        }
        // Riding inside a holder, a weapon fights only if the holder lets it, and
        // then it fights from the holder — which is [`super::garrison`]'s work.
        if world.entity(entity).contains::<HiddenComponent>() {
            continue;
        }
        // A body's guns work only while it operates: a site still going up
        // and a disabled body stand idle.
        match entity_def::operation(world, entity) {
            Operation::Operating => {}
            Operation::UnderConstruction | Operation::Disabled => continue,
        }
        work_body(world, entity, id);
    }
}

/// One body's guns, in the order it mounts them.
fn work_body(world: &mut World, entity: Entity, id: SimulationId) {
    let guns = guns_of(world, entity);
    let ordered = ordered_target(world, entity);
    let spreads = matches!(
        entity_def::of(world, entity).turret_fire,
        TurretFire::Spread
    );
    // A gun hunts on its own initiative, which is its body's stance: a body told
    // to hold its fire holds every gun on it. Whether the body is idle or busy
    // makes no difference — a turret takes no part in where the body goes, so
    // there is nothing for an order to disagree with.
    let hunts = world
        .entity(entity)
        .get::<StanceComponent>()
        .map_or(Stance::Defend, |StanceComponent(stance)| *stance)
        .auto_engages();
    let walking = world.entity(entity).contains::<MoveComponent>();
    let due = acquire::due(id, world.resource::<GameSession>().tick());

    let conditions = Conditions {
        ordered,
        hunts,
        walking,
        due,
        spreads,
    };

    let mut turrets = world
        .entity_mut(entity)
        .take::<TurretsComponent>()
        .expect("a body with turrets carries their state");
    // What the guns before this one took, so a body that spreads its fire can step
    // around itself.
    let mut taken: Vec<SimulationId> = Vec::new();

    for (gun, state) in guns.iter().zip(turrets.0.iter_mut()) {
        let quarry = decide(world, entity, gun, state, &conditions, &taken);
        if let Some(AttackTarget::Entity(target)) = quarry {
            taken.push(target);
        }
        state.switch_quarry(quarry);
        work_turret(world, entity, gun, state);
    }

    world.entity_mut(entity).insert(turrets);
}

/// What every gun on one body shares this tick: the state of the body under them,
/// read once rather than per gun.
struct Conditions {
    /// What an order has named for the whole body to fight, if anything.
    ordered: Option<AttackTarget>,
    /// Whether its stance lets its guns pick their own fights.
    hunts: bool,
    /// Whether it is under way on a walk, which a gun that stops to shoot sits
    /// out — for the whole walk, ticks spent waiting on a blocked cell included.
    walking: bool,
    /// Whether its acquisition scan comes due this tick.
    due: bool,
    /// Whether its guns divide their own targets between them.
    spreads: bool,
}

/// What one gun works this tick.
///
/// An order names a target for the whole body, so a gun that can reach it takes
/// it and does not reconsider. One that cannot — the order is walking the body
/// closer, or this gun does not reach those layers at all — keeps whatever it
/// found for itself, and looks for something when its scan comes due.
fn decide(
    world: &World,
    entity: Entity,
    gun: &Gun,
    state: &TurretState,
    conditions: &Conditions,
    taken: &[SimulationId],
) -> Option<AttackTarget> {
    // A gun that stops to shoot takes no part in a walk.
    if conditions.walking && matches!(gun.conduct, WeaponConduct::Halts) {
        return None;
    }
    if let Some(ordered) = conditions.ordered
        && bears(world, entity, gun, ordered)
    {
        return Some(ordered);
    }
    let notice = entity_def::effective_stat_u32(world, entity, gun.reads.acquire_range);
    // A body told to hold its fire holds every gun on it: a fight the gun picked
    // for itself is given up, not merely left unrenewed.
    let held = match state.quarry {
        Some(AttackTarget::Entity(held))
            if conditions.hunts && acquire::qualifies(world, entity, gun.targets, held, notice) =>
        {
            Some(held)
        }
        _ => None,
    };
    // A gun keeps what it holds unless there is reason to look again: its scan
    // comes due and it has nothing, or it has something and this body spreads its
    // fire — because holding something worth shooting is no reason to leave a
    // second attacker unanswered, which is the whole difference between four guns
    // and one.
    let looks_again = conditions.hunts && conditions.due && (held.is_none() || conditions.spreads);
    if !looks_again {
        return held.map(AttackTarget::Entity);
    }
    // Guns that spread weigh their targets from where each one sits, so the gun
    // facing a new attacker is the one that comes round on it. Guns that focus
    // weigh from the body, which is how they agree on the same target at all.
    let (from, apart_from) = if conditions.spreads {
        (gun_rect(world, entity, gun), taken)
    } else {
        (entity_def::standing_rect(world, entity), &[][..])
    };
    acquire::find_target_apart_from(world, entity, from, gun.targets, notice, apart_from, held)
        .map(AttackTarget::Entity)
}

/// One tick of one gun: it comes round on what it is working, and swings when that
/// is close enough and it bears on it.
fn work_turret(world: &mut World, entity: Entity, gun: &Gun, state: &mut TurretState) {
    let Some(quarry) = state.quarry else {
        return;
    };
    // Fail-closed residue: what a gun holds was checked this tick, so nothing is
    // expected to have gone in between.
    let Some((target, target_position, target_size)) = resolve(world, quarry) else {
        state.switch_quarry(None);
        return;
    };

    // Coming round happens whether or not the shot can be taken, so an approach is
    // spent aiming rather than waiting.
    let off_aim = match chase::bearing_to(world, entity, target_position, target_size) {
        None => 0,
        Some(wanted) => {
            let allowance = turn::units(world, entity, gun.reads.aim_rate);
            state.bearing = state.bearing.turn_toward(wanted, allowance);
            state.bearing.distance(wanted)
        }
    };

    let stats = attack::weapon_stats(
        world,
        entity,
        gun.reads.damage,
        gun.reads.range,
        gun.reads.period,
        gun.reads.damage_point,
    );
    let (position, size) = entity_def::footprint(world, entity);
    // Reach is the body's, not the gun's corner: a target in range of what carries
    // the guns is in range of all of them.
    if !attack::within(
        world,
        position,
        size,
        target_position,
        target_size,
        stats.range,
    ) {
        // Out of reach the cycle waits at its start, as a walking attacker's swing
        // does when its target steps out of range.
        state.phase = 0;
        return;
    }

    // The arc gates the start of a cycle only, as it does for a body's own weapon:
    // a swing already under way was committed when the gun bore on its target.
    let arc = entity_def::effective_stat(world, entity, gun.reads.arc);
    if state.phase == 0 && !attack::bears_on_target(off_aim, arc) {
        return;
    }

    attack::swing(
        world,
        entity,
        FiredFrom::Turret(gun.turret),
        muzzle(position, gun),
        target,
        target_position,
        &stats,
        &mut state.phase,
    );
}

/// The cells a mounted gun sits on, which is what it weighs its own targets from:
/// four guns on one keep all reach as far as the keep does, but the one facing a
/// new attacker is the one that should come round on it.
fn gun_rect(world: &World, entity: Entity, gun: &Gun) -> CellRect {
    let body = entity_def::standing_rect(world, entity);
    CellRect::new(
        CellPos::new(body.origin.x + gun.origin.0, body.origin.y + gun.origin.1),
        gun.size,
    )
}

/// Where a mounted gun's shot leaves from: the middle of the patch it sits on.
fn muzzle(position: FixedUVec2, gun: &Gun) -> FixedUVec2 {
    let half = |cells: u32| FixedU64::from_num(cells) / 2;
    FixedUVec2::new(
        position.x + FixedU64::from_num(gun.origin.0) + half(gun.size.width),
        position.y + FixedU64::from_num(gun.origin.1) + half(gun.size.height),
    )
}

/// What a quarry is to shoot at: a body wherever it now stands, or the cell a
/// shot was sent to. A body that is gone is nothing to work.
fn resolve(world: &World, quarry: AttackTarget) -> Option<(Option<Entity>, FixedUVec2, CellSize)> {
    match quarry {
        AttackTarget::Position(cell) => Some((None, cell, CellSize::ONE)),
        AttackTarget::Entity(id) => {
            let target = world.resource::<EntityIndex>().interactable(world, id)?;
            let (at, size) = entity_def::footprint(world, target);
            Some((Some(target), at, size))
        }
    }
}

/// Whether this gun takes what the body was ordered onto: its own layers reach it,
/// and it is inside the range this gun engages at on its own initiative.
///
/// Judged by that wider range rather than by what it can hit, so a gun brought
/// along on an approach spends it coming round — and one that could never work
/// this target at all is left free to answer whatever is nearer.
fn bears(world: &World, entity: Entity, gun: &Gun, ordered: AttackTarget) -> bool {
    let Some((target, target_position, target_size)) = resolve(world, ordered) else {
        return false;
    };
    match target {
        Some(target) => {
            if !targeting::reaches(gun.targets, entity_def::of(world, target)) {
                return false;
            }
        }
        // A bare cell is worked only by a gun whose shots are sent to one; the
        // rest are left free to answer what has a body.
        None => {
            if !gun.aims_at_cells {
                return false;
            }
        }
    }
    let range = entity_def::effective_stat_u32(world, entity, gun.reads.acquire_range);
    let (position, size) = entity_def::footprint(world, entity);
    attack::within(world, position, size, target_position, target_size, range)
}

/// What the body has been ordered to attack, if anything.
///
/// Read from the whole queue rather than its front entry, because an attack that
/// is walking its body closer sits behind the move it suspended into — and closing
/// is exactly when the guns should already be working it. Only an attack that has
/// been taken up counts: one still queued behind other business has not named a
/// fight yet, and binding the guns to it would have them ignore what is beside
/// the road for something the body has not turned to.
fn ordered_target(world: &World, entity: Entity) -> Option<AttackTarget> {
    world
        .entity(entity)
        .get::<OrderQueueComponent>()?
        .0
        .iter()
        .find_map(|entry| match (&entry.order, entry.state) {
            (Order::Attack { target, .. }, OrderState::InProcessing | OrderState::Suspended) => {
                Some(*target)
            }
            _ => None,
        })
}

/// One mounted gun, flattened for the tick: the handle its shots are fired from,
/// where it sits, what it reaches, which stats it reads, and what it asks of the
/// body.
struct Gun {
    turret: TurretId,
    origin: (u32, u32),
    size: CellSize,
    targets: LayerMask,
    reads: TurretStats,
    conduct: WeaponConduct,
    /// Whether its shots are sent to a place rather than after a body — the only
    /// kind of gun an ordered bare cell binds.
    aims_at_cells: bool,
}

/// Reads a type's guns once, so the loop over them borrows nothing.
fn guns_of(world: &World, entity: Entity) -> Vec<Gun> {
    let registry = world.resource::<ContentRegistry>();
    entity_def::of(world, entity)
        .turrets
        .iter()
        .map(|mount| {
            let turret = registry.turret_def(mount.turret());
            Gun {
                turret: mount.turret(),
                origin: (mount.origin().x, mount.origin().y),
                size: mount.size(),
                targets: turret.weapon().targets(),
                reads: turret.stats(),
                conduct: turret.conduct(),
                aims_at_cells: registry.weapon_aims_at_cells(turret.weapon()),
            }
        })
        .collect()
}
