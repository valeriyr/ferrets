//! Areas compared as declared, and a read area widened by a number of cells.

use ferrets_content::{
    area::{Area, ResolvedArea},
    entity_stats::EntityStatId,
    quantity::Quantity,
};
use ferrets_math::FixedU64;

mod utils;

//
// ─── Known beyond ─────────────────────────────────────────────────────────────
//

#[test]
fn any_gap_is_known_beyond_inside() {
    let gap = Area::Around(Quantity::Stat(EntityStatId::ATTACK_RANGE));
    assert!(gap.known_beyond(Area::Inside));
    assert!(Area::Around(Quantity::from(0)).known_beyond(Area::Inside));
}

#[test]
fn inside_is_never_known_beyond() {
    assert!(!Area::Inside.known_beyond(Area::Inside));
    assert!(!Area::Inside.known_beyond(Area::Around(Quantity::from(0))));
}

#[test]
fn constant_gaps_compare() {
    // 2.5 > 2, and 2 is not beyond 2.
    let wider = Area::Around(Quantity::Constant(utils::fixed("2.5")));
    let bound = Area::Around(Quantity::from(2));
    assert!(wider.known_beyond(bound));
    assert!(!bound.known_beyond(bound));
    assert!(!bound.known_beyond(wider));
}

#[test]
fn gap_read_from_stat_is_not_known_beyond_gap() {
    let stat = Area::Around(Quantity::Stat(EntityStatId::ATTACK_RANGE));
    let constant = Area::Around(Quantity::from(1));
    assert!(!stat.known_beyond(constant));
    assert!(!constant.known_beyond(stat));
}

//
// ─── Widening ─────────────────────────────────────────────────────────────────
//

#[test]
fn inside_widened_by_zero_reaches_what_touches() {
    assert_eq!(
        ResolvedArea::Inside.widened(FixedU64::ZERO),
        ResolvedArea::Around(FixedU64::ZERO)
    );
}

#[test]
fn around_widened_holds_at_integer_limit() {
    // The farthest a width can say, widened further, stays there.
    assert_eq!(
        ResolvedArea::Around(FixedU64::MAX).widened(FixedU64::ONE),
        ResolvedArea::Around(FixedU64::MAX)
    );
}

#[test]
fn around_widened_adds_width() {
    // 1.5 + 2 = 3.5
    assert_eq!(
        ResolvedArea::Around(utils::fixed("1.5")).widened(FixedU64::from_num(2)),
        ResolvedArea::Around(utils::fixed("3.5"))
    );
}
