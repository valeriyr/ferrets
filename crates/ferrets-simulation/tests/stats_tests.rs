//! Tests for the stat store's modifier fold.

use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::components::stats::{Modifier, ModifierOp, StatId, StatsComponent};
use ferrets_simulation::content::registry::ContentRegistry;

//
// ─── Combine ──────────────────────────────────────────────────────────────────
//

#[test]
fn effective_equals_base_with_no_modifiers() {
    let mut store = store(StatId::DAMAGE, 10.0);
    store.recompute(&[]);
    assert_eq!(
        store.effective(StatId::DAMAGE),
        Some(FixedU64::from_num(10))
    );
}

#[test]
fn flat_and_percent_fold_as_base_plus_flat_times_percent() {
    // (10 + 5) * (1 + 0.5) = 22.5
    let mut store = store(StatId::DAMAGE, 10.0);
    store.recompute(&[flat(StatId::DAMAGE, 5.0), percent(StatId::DAMAGE, 0.5)]);
    assert_eq!(
        store.effective(StatId::DAMAGE),
        Some(FixedU64::from_num(22.5))
    );
}

#[test]
fn modifier_order_does_not_change_result() {
    let modifiers = [
        flat(StatId::DAMAGE, 5.0),
        percent(StatId::DAMAGE, 0.5),
        flat(StatId::DAMAGE, 3.0),
        percent(StatId::DAMAGE, -0.2),
    ];
    let mut forward = store(StatId::DAMAGE, 10.0);
    forward.recompute(&modifiers);

    let mut reversed_modifiers = modifiers;
    reversed_modifiers.reverse();
    let mut reversed = store(StatId::DAMAGE, 10.0);
    reversed.recompute(&reversed_modifiers);

    assert_eq!(
        forward.effective(StatId::DAMAGE),
        reversed.effective(StatId::DAMAGE)
    );
}

#[test]
fn negative_percent_is_debuff() {
    // 10 * (1 - 0.5) = 5 (an exactly-representable fraction; non-dyadic percents
    // like 0.4 fold deterministically but carry fixed-point residue).
    let mut store = store(StatId::SPEED, 10.0);
    store.recompute(&[percent(StatId::SPEED, -0.5)]);
    assert_eq!(store.effective(StatId::SPEED), Some(FixedU64::from_num(5)));
}

#[test]
fn effective_clamps_at_zero() {
    // (10 - 20) clamps up to 0 rather than going negative.
    let mut store = store(StatId::MAX_HEALTH, 10.0);
    store.recompute(&[flat(StatId::MAX_HEALTH, -20.0)]);
    assert_eq!(store.effective(StatId::MAX_HEALTH), Some(FixedU64::ZERO));
}

#[test]
fn modifiers_for_absent_stats_are_ignored() {
    let mut store = store(StatId::DAMAGE, 10.0);
    store.recompute(&[flat(StatId::ARMOR, 5.0)]);
    assert_eq!(
        store.effective(StatId::DAMAGE),
        Some(FixedU64::from_num(10))
    );
    assert_eq!(store.effective(StatId::ARMOR), None);
}

//
// ─── Floors ───────────────────────────────────────────────────────────────────
//

#[test]
fn floored_stat_holds_at_its_floor() {
    // The attack cycle counts whole ticks and the hit lands on a phase inside it,
    // so a debuff deep enough to zero the period still leaves one tick.
    let mut store = store(StatId::ATTACK_PERIOD, 6.0);
    store.recompute(&[flat(StatId::ATTACK_PERIOD, -10.0)]);
    assert_eq!(store.effective(StatId::ATTACK_PERIOD), Some(FixedU64::ONE));
}

#[test]
fn fractional_stat_is_not_raised_to_whole_number() {
    // Speed is fractional grid units per tick and authored below 1, so it carries
    // no floor — folding must leave it exactly where the modifiers put it.
    let mut store = store(StatId::SPEED, 0.3);
    store.recompute(&[]);
    assert_eq!(
        store.effective(StatId::SPEED),
        Some(FixedU64::from_num(0.3))
    );

    store.recompute(&[percent(StatId::SPEED, 1.0)]);
    assert_eq!(
        store.effective(StatId::SPEED),
        Some(FixedU64::from_num(0.6))
    );
}

#[test]
fn unfloored_stat_folds_to_zero() {
    // Armor is meaningful at zero — it simply means no mitigation.
    let mut store = store(StatId::ARMOR, 5.0);
    store.recompute(&[flat(StatId::ARMOR, -10.0)]);
    assert_eq!(store.effective(StatId::ARMOR), Some(FixedU64::ZERO));
}

#[test]
fn custom_stat_folds_to_zero() {
    // The engine has no semantics for a content-declared stat, so it never
    // imposes a floor on one.
    let morale = ContentRegistry::default().register_stat("morale");
    let mut store = store(morale, 5.0);
    store.recompute(&[flat(morale, -10.0)]);
    assert_eq!(store.effective(morale), Some(FixedU64::ZERO));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn flat(stat: StatId, magnitude: f64) -> Modifier {
    Modifier {
        stat,
        op: ModifierOp::FlatAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn percent(stat: StatId, magnitude: f64) -> Modifier {
    Modifier {
        stat,
        op: ModifierOp::PercentAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn store(stat: StatId, base: f64) -> StatsComponent {
    let mut store = StatsComponent::default();
    store.set_base(stat, FixedU64::from_num(base));
    store
}
