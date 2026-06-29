//! Per-player resource stockpiles: stocking, spending, affording, and refunding.

use ferrets_simulation::resources::{self, PlayerResources};

//
// ─── Stocking ───────────────────────────────────────────────────────────────
//

#[test]
fn fresh_stockpile_is_empty() {
    let resources = PlayerResources::new(2);

    assert_eq!(resources.amount(0, "gold"), 0);
    assert_eq!(resources.amount(1, "wood"), 0);
}

#[test]
fn add_accumulates() {
    let mut resources = PlayerResources::new(1);

    resources.add(0, "gold", 100);
    resources.add(0, "gold", 50);

    assert_eq!(resources.amount(0, "gold"), 150);
}

#[test]
fn add_saturates_at_max() {
    let mut resources = PlayerResources::new(1);

    resources.add(0, "gold", u32::MAX);
    resources.add(0, "gold", 10);

    assert_eq!(resources.amount(0, "gold"), u32::MAX);
}

#[test]
fn stockpiles_are_per_player_and_per_kind() {
    let mut resources = PlayerResources::new(2);

    resources.add(0, "gold", 100);
    resources.add(1, "wood", 200);

    assert_eq!(resources.amount(0, "gold"), 100);
    assert_eq!(resources.amount(0, "wood"), 0);
    assert_eq!(resources.amount(1, "gold"), 0);
    assert_eq!(resources.amount(1, "wood"), 200);
}

//
// ─── Affording ──────────────────────────────────────────────────────────────
//

#[test]
fn can_afford_when_every_kind_is_covered() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 100);
    resources.add(0, "wood", 50);

    assert!(resources.can_afford(0, &resources::cost([("gold", 100), ("wood", 50)])));
    assert!(resources.can_afford(0, &resources::cost([("gold", 80)])));
}

#[test]
fn cannot_afford_when_any_kind_is_short() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 100);

    // Enough gold, but wood is missing entirely (treated as 0).
    assert!(!resources.can_afford(0, &resources::cost([("gold", 100), ("wood", 1)])));
    assert!(!resources.can_afford(0, &resources::cost([("gold", 101)])));
}

#[test]
fn exact_amount_is_affordable() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 100);

    assert!(resources.can_afford(0, &resources::cost([("gold", 100)])));
}

//
// ─── Spending and refunding ───────────────────────────────────────────────────
//

#[test]
fn subtract_deducts_each_kind() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 100);
    resources.add(0, "wood", 50);

    resources.subtract(0, &resources::cost([("gold", 30), ("wood", 50)]));

    assert_eq!(resources.amount(0, "gold"), 70);
    assert_eq!(resources.amount(0, "wood"), 0);
}

#[test]
#[should_panic(expected = "cannot afford")]
fn subtracting_more_than_owned_panics() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 10);

    resources.subtract(0, &resources::cost([("gold", 11)]));
}

#[test]
fn refund_returns_cost_to_stockpile() {
    let mut resources = PlayerResources::new(1);
    resources.add(0, "gold", 100);
    let price = resources::cost([("gold", 40), ("wood", 20)]);

    resources.subtract(0, &resources::cost([("gold", 40)]));
    resources.refund(0, &price);

    // The gold spent is back, and the refunded wood is stocked even though it was
    // never spent (refund just adds).
    assert_eq!(resources.amount(0, "gold"), 100);
    assert_eq!(resources.amount(0, "wood"), 20);
}

//
// ─── Iteration ────────────────────────────────────────────────────────────────
//

#[test]
fn iter_yields_present_kinds_in_deterministic_order() {
    let mut resources = PlayerResources::new(2);
    // Insert out of order to prove iteration is sorted, not insertion-ordered.
    resources.add(1, "wood", 5);
    resources.add(0, "wood", 2);
    resources.add(0, "gold", 1);

    let entries: Vec<_> = resources.iter().collect();

    // Ascending player, then kind; players with no stock contribute nothing.
    assert_eq!(
        entries,
        vec![(0, "gold", 1), (0, "wood", 2), (1, "wood", 5)],
    );
}
