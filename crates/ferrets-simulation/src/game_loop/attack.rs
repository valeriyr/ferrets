//! Attack order implementation: the weapon a body points itself.
//! Called by [`super::orders`] as part of the shared order lifecycle.
//!
//! A body has one look and its walk owns it, so a weapon pointed by the body stops
//! to shoot: this order closes the distance and then works the weapon where it
//! stands. The turrets a body carries are not worked from here — they have
//! bearings of their own and [`super::turret`] works them, including while this
//! order is walking the body somewhere.

use bevy_ecs::{entity::Entity, world::World};
use ferrets_content::{entity_stats::EntityStatId, registry::ContentRegistry, targeting};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, facing, fixed_uvec2::FixedUVec2};
use ferrets_physics::body;

use super::{
    chase::{self, Destination},
    impacts,
    orders::Processing,
    turn,
};
use crate::{
    components::{
        attack::AttackComponent,
        entity_stats::StatsComponent,
        order_queue::{CancelPolicy, OrderState},
    },
    entity_def,
    entity_index::EntityIndex,
    impacts::FiredFrom,
    map::Map,
    order::{AttackTarget, Order},
};

/// What a weapon is fighting by this tick: its four measurements as the
/// modifier pipeline leaves them.
pub(super) struct WeaponStats {
    /// How far the weapon reaches, in cells.
    pub(super) range: u32,
    /// The damage one landed hit carries.
    pub(super) damage: FixedU64,
    /// Ticks in one full attack cycle.
    pub(super) attack_period: u32,
    /// The tick within the cycle the hit lands on.
    pub(super) damage_point: u32,
}

/// Called once when an Attack order becomes the front `New` entry.
///
/// Inserts the fight state and returns `InProcessing`, or `Finished` immediately
/// if the entity cannot attack or the target is no longer alive.
pub fn prepare(entity: Entity, order: &Order, world: &mut World) -> OrderState {
    let target = order
        .attack_target()
        .expect("Attack order must have a target");

    if !entity_def::of(world, entity).can_attack() {
        return OrderState::Finished;
    }
    // A cell is always there to be shelled; only a named entity can already be gone.
    if let Some(id) = target.entity()
        && world
            .resource::<EntityIndex>()
            .interactable(world, id)
            .is_none()
    {
        return OrderState::Finished;
    }

    world.entity_mut(entity).insert(AttackComponent::default());
    OrderState::InProcessing
}

/// Called when an Attack order resumes from `Suspended` (its chase move just
/// finished). The fight state survives suspension — a weapon that fights on the
/// move has been working it the whole way — and validation happens in [`process`].
pub fn prepare_suspended(_entity: Entity, _order: &Order, _world: &mut World) -> OrderState {
    OrderState::InProcessing
}

/// Called for every Attack entry that has a cancel policy.
///
/// An attack stops immediately under both policies: the fight is taken off the
/// entity and the entry finishes. A swing in progress is simply abandoned.
pub fn cancel_processing(
    entity: Entity,
    _order: &Order,
    _policy: CancelPolicy,
    _entry_state: OrderState,
    world: &mut World,
) -> OrderState {
    world.entity_mut(entity).remove::<AttackComponent>();
    OrderState::Finished
}

/// Advance an Attack order by one tick.
///
/// Each tick:
/// 1. If the target is gone or dying, the order finishes. A leashed attack also
///    finishes when the target has strayed beyond the leash.
/// 2. If the target is out of range, the swing resets and a chase move toward the
///    target's current position is requested as a sub-order (the entry suspends).
///    The order finishes instead when the previous chase ended without the entity
///    getting any closer — the target is unreachable.
/// 3. Otherwise the weapon is worked: it comes round on the target, and the swing
///    advances when it bears — the hit lands as the phase reaches `damage_point`,
///    and the cycle restarts at `attack_period`. The turrets the body carries are
///    not worked from here; they have been working this target since the order
///    named it, from wherever the walk had reached (see [`super::turret`]).
///
/// A target killed by the landed hit starts dying immediately; the order itself
/// finishes on the next tick when the target is no longer alive.
pub fn process(entity: Entity, order: &Order, world: &mut World) -> Processing {
    let target_aim = order
        .attack_target()
        .expect("Attack order must have a target");

    let Some(mut attack_component) = world.entity_mut(entity).take::<AttackComponent>() else {
        return Processing::state(OrderState::Finished);
    };

    // A named entity must still be reachable and is chased, measured and faced by
    // its whole footprint wherever it goes; a cell is simply where it was aimed, has
    // no footprint of its own, and needs no such check.
    let (target, target_position, target_size) = match target_aim {
        AttackTarget::Entity(id) => {
            let Some(target) = world.resource::<EntityIndex>().interactable(world, id) else {
                return Processing::state(OrderState::Finished);
            };
            // Reachability is re-judged every tick, not only at issue, because a
            // form change moves the answer: a target that takes off mid-fight
            // leaves this weapon's layers, and chasing it would swing forever at
            // something no hit can land on.
            if !targeting::reaches(
                entity_def::weapon_targets(world, entity),
                entity_def::of(world, target),
            ) {
                return Processing::state(OrderState::Finished);
            }
            let (at, size) = entity_def::footprint(world, target);
            (Some(target), at, size)
        }
        AttackTarget::Position(cell) => (None, cell, CellSize::ONE),
    };

    let (position, size) = entity_def::footprint(world, entity);
    // How far the body must be walked is the longest reach it has among the
    // weapons that can serve this target, wherever each is fitted: an order is
    // given to the entity, and a keep's guns are as much a reason to stop walking
    // as a spear in its own hands — but a gun that could never join this fight is
    // no reason to stop short of the one that can.
    let range = entity_def::weapon_range_serving(world, entity, target);

    if let Some(leash) = order.attack_leash() {
        // Footprint-based like every range check, and judged exactly like the
        // chase below — the cells the body stood on when it was leashed reach
        // for the target's own footprint — because a mid-cell target must not
        // read as leashed to the leash and out of range to the chase, or the
        // one extra walk breaks a stand-ground stance's never-move promise.
        if !world.resource::<Map>().projection().in_range_for_rects(
            body::standing_rect(leash.anchor, size),
            CellRect::new(CellPos::from(target_position), target_size),
            leash.radius,
        ) {
            return Processing::state(OrderState::Finished);
        }
    }

    // A gun that cannot close does not chase: it brings itself to bear and waits
    // for the target to come inside reach. Bounded by what it would engage on its
    // own initiative, so it tracks what it might yet shoot and lets go of the rest
    // — and never consults the chase, whose patience is about walks that get
    // nowhere and would end a fight a turret is right to be holding.
    if !entity_def::of(world, entity).can_move()
        && !within(world, position, size, target_position, target_size, range)
    {
        let notice = entity_def::notice_range_serving(world, entity, target);
        if !within(world, position, size, target_position, target_size, notice) {
            return Processing::state(OrderState::Finished);
        }
        // The swing resets while out of range, as it does for a walker, and the
        // body still comes round on what it is waiting for — when a weapon of its
        // own is what turns with it. A body that fights only from turrets has no
        // look in the fight: its guns bear on their own and its walls stay put.
        attack_component.phase = 0;
        if entity_def::of(world, entity).attack.is_some()
            && let Some(wanted) = chase::bearing_to(world, entity, target_position, target_size)
        {
            turn::toward(world, entity, wanted, turn::Rate::Standing);
        }
        world.entity_mut(entity).insert(attack_component);
        return Processing::state(OrderState::InProcessing);
    }

    let destination = match target {
        Some(target) => chase::advance_to_entity(
            &mut attack_component.last_chase,
            world,
            entity,
            target,
            range,
        ),
        None => chase::advance(
            &mut attack_component.last_chase,
            world.resource::<Map>().projection(),
            position,
            size,
            target_position,
            target_size,
            range,
        ),
    };
    match destination {
        Destination::OutOfReach => return Processing::state(OrderState::Finished),
        Destination::Walk(move_order) => {
            // The swing resets while out of range.
            attack_component.phase = 0;
            world.entity_mut(entity).insert(attack_component);
            return Processing::suspend(move_order);
        }
        Destination::Arrived => {}
    }

    // Arrived. What happens now is the body's own weapon's business — a type that
    // fights only from turrets has nothing to do here, because its guns have been
    // working this target from [`super::turret`] all along.
    if entity_def::of(world, entity).attack.is_none() {
        world.entity_mut(entity).insert(attack_component);
        return Processing::state(OrderState::InProcessing);
    }
    let stats = body_weapon_stats(world, entity);

    // Arrival was judged by the body's longest reach, which may be a turret's.
    // Its own weapon still fires only inside its own range, at layers it reaches
    // itself, and at a bare cell only when its shots are sent to one — otherwise
    // it holds while the guns it arrived for keep working from [`super::turret`].
    let body_serves = match target {
        Some(target) => targeting::reaches(
            entity_def::body_weapon_targets(world, entity),
            entity_def::of(world, target),
        ),
        None => {
            let registry = world.resource::<ContentRegistry>();
            let weapon = entity_def::of(world, entity)
                .attack
                .as_ref()
                .expect("the body points a weapon")
                .weapon();
            registry.weapon_aims_at_cells(weapon)
        }
    };
    if !body_serves
        || !within(
            world,
            position,
            size,
            target_position,
            target_size,
            stats.range,
        )
    {
        // Out of its own reach the swing waits at its start, as every weapon's
        // does, and the body still comes round on what it is waiting for.
        attack_component.phase = 0;
        if let Some(wanted) = chase::bearing_to(world, entity, target_position, target_size) {
            turn::toward(world, entity, wanted, turn::Rate::Standing);
        }
        world.entity_mut(entity).insert(attack_component);
        return Processing::state(OrderState::InProcessing);
    }

    // How far the body still points off its target once it has come round this
    // tick, which is what the arc is judged against.
    let off_aim = match chase::bearing_to(world, entity, target_position, target_size) {
        None => 0,
        Some(wanted) => turn::toward(world, entity, wanted, turn::Rate::Standing).distance(wanted),
    };

    // The arc gates the start of a cycle only. One already under way always
    // lands: the shot was committed when the weapon bore on the target, and a
    // target that wanders out mid-swing has already been fired at — which is also
    // why nothing here needs the hysteresis a continuous check would.
    let arc = entity_def::effective_stat(world, entity, EntityStatId::ATTACK_ARC);
    if attack_component.phase == 0 && !bears_on_target(off_aim, arc) {
        world.entity_mut(entity).insert(attack_component);
        return Processing::state(OrderState::InProcessing);
    }

    swing(
        world,
        entity,
        FiredFrom::Body,
        entity_def::footprint_center(world, entity),
        target,
        target_position,
        &stats,
        &mut attack_component.phase,
    );

    world.entity_mut(entity).insert(attack_component);
    Processing::state(OrderState::InProcessing)
}

/// Whether `range` reaches from the attacker's footprint to its target's, judged
/// exactly as the chase judges arrival — both sides by their footprints, so a wide
/// attacker reaches from its nearest edge.
pub(super) fn within(
    world: &World,
    position: FixedUVec2,
    size: CellSize,
    target_position: FixedUVec2,
    target_size: CellSize,
    range: u32,
) -> bool {
    world.resource::<Map>().projection().in_range_for_rects(
        body::standing_rect(position, size),
        CellRect::new(CellPos::from(target_position), target_size),
        range,
    )
}

/// Whether a weapon pointing `off_aim` angle units away from its target may fire
/// through `arc`.
///
/// A weapon with no arc fires wherever it points; one with an arc fires through
/// half of it either side of where it points, so a gun that must be brought to
/// bear holds its fire until it is.
pub(super) fn bears_on_target(off_aim: u32, arc: Option<FixedU64>) -> bool {
    match arc {
        None => true,
        Some(arc) => off_aim * 2 <= facing::units_of_degrees(arc),
    }
}

/// Reads the numbers the body's own weapon fights by: the standard stats, which
/// are that weapon's by definition — only a turret names its own.
pub(super) fn body_weapon_stats(world: &World, entity: Entity) -> WeaponStats {
    weapon_stats(
        world,
        entity,
        EntityStatId::DAMAGE,
        EntityStatId::ATTACK_RANGE,
        EntityStatId::ATTACK_PERIOD,
        EntityStatId::DAMAGE_POINT,
    )
}

/// Reads the numbers a weapon fights by, fresh for this tick so modifiers land,
/// each from the stat the caller names for it.
///
/// Panics if the entity does not carry those stats, which registration refuses.
pub(super) fn weapon_stats(
    world: &World,
    entity: Entity,
    damage: EntityStatId,
    range: EntityStatId,
    period: EntityStatId,
    damage_point: EntityStatId,
) -> WeaponStats {
    let stats = world
        .entity(entity)
        .get::<StatsComponent>()
        .expect("attackers have a stat store");
    let range = stats.effective_as_u32(range).unwrap();
    let damage = stats.effective(damage).unwrap();
    let attack_period = stats.effective_as_u32(period).unwrap();
    // Registration keeps the authored damage point inside the authored cycle, but
    // the two stats take modifiers independently, so a shortened cycle can leave
    // the hit beyond its end — where the phase counter would never reach it.
    let damage_point = stats
        .effective_as_u32(damage_point)
        .unwrap()
        .min(attack_period);

    WeaponStats {
        range,
        damage,
        attack_period,
        damage_point,
    }
}

/// Advances one swing by a tick, on the numbers `stats` gives: the hit leaves
/// `origin` when the phase reaches the damage point, and the cycle restarts at its
/// end.
#[allow(clippy::too_many_arguments)]
pub(super) fn swing(
    world: &mut World,
    attacker: Entity,
    fired_from: FiredFrom,
    origin: FixedUVec2,
    target: Option<Entity>,
    aimed_at: FixedUVec2,
    stats: &WeaponStats,
    phase: &mut u32,
) {
    *phase += 1;

    if *phase == stats.damage_point {
        impacts::deliver(
            world,
            attacker,
            fired_from,
            origin,
            target,
            aimed_at,
            stats.damage,
        );
    }

    // At least, not exactly at: a cycle stat can shrink under a phase already
    // counted — a debuff mid-swing, a morph carrying a gun's state into a shorter
    // cycle — and an exact test would count past the end forever, a gun jammed
    // for good with nothing to say so.
    if *phase >= stats.attack_period {
        *phase = 0;
    }
}
