//! Tests for the per-player stat store's modifier fold.

use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    content::{
        entity_stats::EntityStatId,
        player_stats::PlayerStatId,
        registry::ContentRegistry,
        stats::{EntityModifier, ModifierOp, PlayerModifier},
    },
    player_stats::PlayerStats,
};

//
// ─── Base values ──────────────────────────────────────────────────────────────
//

#[test]
fn set_base_reads_back_through_base_and_effective() {
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(20));

    assert_eq!(
        stats.base(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(20))
    );
    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(20))
    );
}

#[test]
fn absent_stat_reads_none() {
    let stats = PlayerStats::new(1);

    assert_eq!(stats.base(0, PlayerStatId::MAX_SUPPLY), None);
    assert_eq!(stats.effective(0, PlayerStatId::MAX_SUPPLY), None);
    assert_eq!(stats.effective_as_u32(0, PlayerStatId::MAX_SUPPLY), None);
}

#[test]
fn effective_as_u32_truncates_fractional_value() {
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(12.75));

    assert_eq!(
        stats.effective_as_u32(0, PlayerStatId::MAX_SUPPLY),
        Some(12)
    );
}

//
// ─── Modifiers ────────────────────────────────────────────────────────────────
//

#[test]
fn flat_and_percent_fold_as_base_plus_flat_times_percent() {
    // (10 + 5) * (1 + 0.5) = 22.5
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(10));
    stats.add_player_modifier(0, flat(PlayerStatId::MAX_SUPPLY, 5.0));
    stats.add_player_modifier(0, percent(PlayerStatId::MAX_SUPPLY, 0.5));

    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(22.5))
    );
    // The base stays what was set; only the effective value folds.
    assert_eq!(
        stats.base(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(10))
    );
}

#[test]
fn effective_clamps_at_zero() {
    // (10 - 20) clamps up to 0 rather than going negative.
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(10));
    stats.add_player_modifier(0, flat(PlayerStatId::MAX_SUPPLY, -20.0));

    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::ZERO)
    );
}

#[test]
fn remove_modifier_refolds_to_base() {
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(10));

    let bonus = flat(PlayerStatId::MAX_SUPPLY, 5.0);
    stats.add_player_modifier(0, bonus);
    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(15))
    );

    stats.remove_player_modifier(0, bonus);
    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(10))
    );
}

#[test]
fn modifier_over_absent_stat_is_inert_until_base_arrives() {
    let morale = ContentRegistry::default().register_player_stat("morale");
    let mut stats = PlayerStats::new(1);

    stats.add_player_modifier(0, flat(morale, 5.0));
    assert_eq!(stats.base(0, morale), None);
    assert_eq!(stats.effective(0, morale), None);

    stats.set_base(0, morale, FixedU64::from_num(10));
    assert_eq!(stats.effective(0, morale), Some(FixedU64::from_num(15)));
}

//
// ─── Player isolation ─────────────────────────────────────────────────────────
//

#[test]
fn player_stats_do_not_leak_between_players() {
    let mut stats = PlayerStats::new(2);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(10));
    stats.add_player_modifier(0, flat(PlayerStatId::MAX_SUPPLY, 5.0));

    assert_eq!(stats.base(1, PlayerStatId::MAX_SUPPLY), None);
    assert_eq!(stats.effective(1, PlayerStatId::MAX_SUPPLY), None);

    // Player 0's modifier folds into player 0's stat only.
    stats.set_base(1, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(20));
    assert_eq!(
        stats.effective(1, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(20))
    );
    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(15))
    );
}

//
// ─── Entity modifiers ─────────────────────────────────────────────────────────
//

#[test]
fn entity_modifier_reads_back_without_touching_player_stats() {
    let mut stats = PlayerStats::new(1);
    stats.set_base(0, PlayerStatId::MAX_SUPPLY, FixedU64::from_num(10));
    assert!(stats.entity_modifiers(0).is_empty());

    let boost = entity_percent(EntityStatId::SPEED, 1.0);
    stats.add_entity_modifier(0, boost);

    assert_eq!(stats.entity_modifiers(0), &[boost]);
    // The player store never folds entity modifiers; they reach the player's
    // units at the entity recompute instead.
    assert_eq!(
        stats.effective(0, PlayerStatId::MAX_SUPPLY),
        Some(FixedU64::from_num(10))
    );
}

#[test]
fn remove_entity_modifier_removes_one_instance_at_time() {
    let mut stats = PlayerStats::new(1);
    let boost = entity_flat(EntityStatId::SPEED, 0.5);
    stats.add_entity_modifier(0, boost);
    stats.add_entity_modifier(0, boost);

    // Two identical instances come off one per removal.
    stats.remove_entity_modifier(0, boost);
    assert_eq!(stats.entity_modifiers(0), &[boost]);

    stats.remove_entity_modifier(0, boost);
    assert!(stats.entity_modifiers(0).is_empty());

    // Removing what is not applied changes nothing.
    stats.remove_entity_modifier(0, boost);
    assert!(stats.entity_modifiers(0).is_empty());
}

#[test]
fn entity_modifiers_do_not_leak_between_players() {
    let mut stats = PlayerStats::new(2);
    let boost = entity_percent(EntityStatId::SPEED, 1.0);
    stats.add_entity_modifier(0, boost);

    assert_eq!(stats.entity_modifiers(0), &[boost]);
    assert!(stats.entity_modifiers(1).is_empty());
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn entity_flat(stat: EntityStatId, magnitude: f64) -> EntityModifier {
    EntityModifier {
        stat,
        op: ModifierOp::FlatAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn entity_percent(stat: EntityStatId, magnitude: f64) -> EntityModifier {
    EntityModifier {
        stat,
        op: ModifierOp::PercentAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn flat(stat: PlayerStatId, magnitude: f64) -> PlayerModifier {
    PlayerModifier {
        stat,
        op: ModifierOp::FlatAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn percent(stat: PlayerStatId, magnitude: f64) -> PlayerModifier {
    PlayerModifier {
        stat,
        op: ModifierOp::PercentAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}
