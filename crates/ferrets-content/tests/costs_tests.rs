//! The [`Cost`] builder: entry conversion into the ordered price map.

use ferrets_content::costs::{self, Cost};

#[test]
fn cost_collects_entries_into_map() {
    let cost = costs::cost([("gold", 100), ("wood", 50)]);

    assert_eq!(
        cost,
        Cost::from([("gold".to_string(), 100), ("wood".to_string(), 50)])
    );
}

#[test]
fn cost_without_entries_is_empty() {
    let cost = costs::cost(Vec::<(String, u32)>::new());

    assert!(cost.is_empty());
}

#[test]
fn cost_keeps_last_amount_for_repeated_kind() {
    let cost = costs::cost([("gold", 100), ("gold", 25)]);

    assert_eq!(cost, Cost::from([("gold".to_string(), 25)]));
}
