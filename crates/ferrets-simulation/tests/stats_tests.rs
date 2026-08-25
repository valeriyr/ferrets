//! Tests for the stat store's modifier fold.

use ferrets_content::{
    entity_stats::EntityStatId,
    player_stats::PlayerStatId,
    registry::ContentRegistry,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::components::entity_stats::StatsComponent;

//
// ─── Combine ──────────────────────────────────────────────────────────────────
//

#[test]
fn effective_equals_base_with_no_modifiers() {
    let mut store = store(EntityStatId::DAMAGE, "10");
    store.recompute(&[]);
    assert_eq!(
        store.effective(EntityStatId::DAMAGE),
        Some(FixedU64::from_num(10))
    );
}

#[test]
fn flat_and_percent_fold_as_base_plus_flat_times_percent() {
    // (10 + 5) * (1 + 0.5) = 22.5
    let mut store = store(EntityStatId::DAMAGE, "10");
    store.recompute(&[
        flat(EntityStatId::DAMAGE, "5"),
        percent(EntityStatId::DAMAGE, "0.5"),
    ]);
    assert_eq!(
        store.effective(EntityStatId::DAMAGE),
        Some(FixedU64::from_num(22.5))
    );
}

#[test]
fn modifier_order_does_not_change_result() {
    let modifiers = [
        flat(EntityStatId::DAMAGE, "5"),
        percent(EntityStatId::DAMAGE, "0.5"),
        flat(EntityStatId::DAMAGE, "3"),
        percent(EntityStatId::DAMAGE, "-0.2"),
    ];
    let mut forward = store(EntityStatId::DAMAGE, "10");
    forward.recompute(&modifiers);

    let mut reversed_modifiers = modifiers;
    reversed_modifiers.reverse();
    let mut reversed = store(EntityStatId::DAMAGE, "10");
    reversed.recompute(&reversed_modifiers);

    assert_eq!(
        forward.effective(EntityStatId::DAMAGE),
        reversed.effective(EntityStatId::DAMAGE)
    );
}

#[test]
fn negative_percent_is_debuff() {
    // 10 * (1 - 0.5) = 5 (an exactly-representable fraction; non-dyadic percents
    // like 0.4 fold deterministically but carry fixed-point residue).
    let mut store = store(EntityStatId::SPEED, "10");
    store.recompute(&[percent(EntityStatId::SPEED, "-0.5")]);
    assert_eq!(
        store.effective(EntityStatId::SPEED),
        Some(FixedU64::from_num(5))
    );
}

#[test]
fn effective_clamps_at_zero() {
    // (10 - 20) clamps up to 0 rather than going negative. Read on an unfloored
    // stat, so the clamp is what the result shows.
    let mut store = store(EntityStatId::DAMAGE, "10");
    store.recompute(&[flat(EntityStatId::DAMAGE, "-20")]);
    assert_eq!(store.effective(EntityStatId::DAMAGE), Some(FixedU64::ZERO));
}

#[test]
fn modifiers_for_absent_stats_are_ignored() {
    let mut store = store(EntityStatId::DAMAGE, "10");
    store.recompute(&[flat(EntityStatId::ARMOR, "5")]);
    assert_eq!(
        store.effective(EntityStatId::DAMAGE),
        Some(FixedU64::from_num(10))
    );
    assert_eq!(store.effective(EntityStatId::ARMOR), None);
}

//
// ─── Floors ───────────────────────────────────────────────────────────────────
//

#[test]
fn floored_stat_holds_at_its_floor() {
    // The attack cycle counts whole ticks and the hit lands on a phase inside it,
    // so a debuff deep enough to zero the period still leaves one tick.
    let mut store = store(EntityStatId::ATTACK_PERIOD, "6");
    store.recompute(&[flat(EntityStatId::ATTACK_PERIOD, "-10")]);
    assert_eq!(
        store.effective(EntityStatId::ATTACK_PERIOD),
        Some(FixedU64::ONE)
    );
}

#[test]
fn pool_ceiling_holds_at_its_floor() {
    // Current health settles under max_health, so a debuff deep enough to zero the
    // ceiling would be an instant kill; the floor leaves a point to stand on.
    let mut store = store(EntityStatId::MAX_HEALTH, "40");
    store.recompute(&[percent(EntityStatId::MAX_HEALTH, "-1")]);
    assert_eq!(
        store.effective(EntityStatId::MAX_HEALTH),
        Some(FixedU64::ONE)
    );
}

#[test]
fn fractional_stat_is_not_raised_to_whole_number() {
    // Speed is fractional grid units per tick and authored below 1, so it carries
    // no floor — folding must leave it exactly where the modifiers put it.
    let mut store = store(EntityStatId::SPEED, "0.3");
    store.recompute(&[]);
    assert_eq!(
        store.effective(EntityStatId::SPEED),
        Some(FixedU64::from_num(0.3))
    );

    store.recompute(&[percent(EntityStatId::SPEED, "1")]);
    assert_eq!(
        store.effective(EntityStatId::SPEED),
        Some(FixedU64::from_num(0.6))
    );
}

#[test]
fn unfloored_stat_folds_to_zero() {
    // Armor is meaningful at zero — it simply means no mitigation.
    let mut store = store(EntityStatId::ARMOR, "5");
    store.recompute(&[flat(EntityStatId::ARMOR, "-10")]);
    assert_eq!(store.effective(EntityStatId::ARMOR), Some(FixedU64::ZERO));
}

#[test]
fn working_reaches_hold_at_their_floor() {
    // Every distance a capability reaches for is a stat, and none of them mean
    // anything at zero: a reach of nothing could only be satisfied by standing
    // inside the target's own footprint, which no solid one allows. A debuff deep
    // enough to zero one leaves the worker able to touch what is next to it.
    for reach in [
        EntityStatId::BUILD_RANGE,
        EntityStatId::REPAIR_RANGE,
        EntityStatId::HARVEST_RANGE,
    ] {
        let mut store = store(reach, "3");
        store.recompute(&[flat(reach, "-10")]);
        assert_eq!(
            store.effective(reach),
            Some(FixedU64::ONE),
            "{reach:?} folded past its floor"
        );
    }
}

#[test]
fn harvest_range_folds_like_any_other_reach() {
    let mut store = store(EntityStatId::HARVEST_RANGE, "2");

    store.recompute(&[]);
    assert_eq!(
        store.effective(EntityStatId::HARVEST_RANGE),
        Some(FixedU64::from_num(2)),
        "an unmodified reach is what content authored"
    );

    // Whole cells, so a fractional result is what the reader truncates, not what
    // the store rounds — `effective_as_u32` is what range checks actually consume.
    store.recompute(&[percent(EntityStatId::HARVEST_RANGE, "0.75")]);
    assert_eq!(
        store.effective(EntityStatId::HARVEST_RANGE),
        Some(FixedU64::from_num(3.5))
    );
    assert_eq!(store.effective_as_u32(EntityStatId::HARVEST_RANGE), Some(3));
}

#[test]
fn built_in_stats_are_registered_under_their_content_names() {
    // Content authors reach each stat by name, so the name has to resolve to the
    // same handle the consuming order reads.
    let registry = ContentRegistry::default();

    for (name, stat) in [
        ("harvest_range", EntityStatId::HARVEST_RANGE),
        ("build_range", EntityStatId::BUILD_RANGE),
        ("repair_range", EntityStatId::REPAIR_RANGE),
        ("repair_speed", EntityStatId::REPAIR_SPEED),
        ("repair_cost_factor", EntityStatId::REPAIR_COST_FACTOR),
        ("health_regen", EntityStatId::HEALTH_REGEN),
        ("supply_provided", EntityStatId::SUPPLY_PROVIDED),
        ("supply_cost", EntityStatId::SUPPLY_COST),
    ] {
        assert_eq!(
            registry.entity_stat(name),
            Some(stat),
            "'{name}' did not resolve to {stat:?}"
        );
    }
}

#[test]
fn built_in_player_stats_are_registered_under_their_content_names() {
    let registry = ContentRegistry::default();

    assert_eq!(
        registry.player_stat("max_supply"),
        Some(PlayerStatId::MAX_SUPPLY)
    );
}

#[test]
fn custom_stat_folds_to_zero() {
    // The engine has no semantics for a content-declared stat, so it never
    // imposes a floor on one.
    let morale = ContentRegistry::default().register_entity_stat("morale");
    let mut store = store(morale, "5");
    store.recompute(&[flat(morale, "-10")]);
    assert_eq!(store.effective(morale), Some(FixedU64::ZERO));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn flat(stat: EntityStatId, magnitude: &str) -> EntityModifier {
    EntityModifier {
        stat,
        op: ModifierOp::FlatAdd,
        magnitude: signed_value(magnitude),
    }
}

fn percent(stat: EntityStatId, magnitude: &str) -> EntityModifier {
    EntityModifier {
        stat,
        op: ModifierOp::PercentAdd,
        magnitude: signed_value(magnitude),
    }
}

/// A value written as decimal digits rather than a float, so the number the
/// digits name is the one the store holds.
fn value(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a stat value"))
}

/// The same, where the value can move a stat down as well as up.
fn signed_value(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a modifier magnitude"))
}

fn store(stat: EntityStatId, base: &str) -> StatsComponent {
    let mut store = StatsComponent::default();
    store.set_base(stat, value(base));
    store
}
