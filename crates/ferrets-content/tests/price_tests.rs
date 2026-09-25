//! The [`Price`] builder: entries collected into the ordered price map.

use ferrets_content::price::{self, Price};

#[test]
fn from_collects_entries_into_map() {
    let price = price::from([("gold", 100), ("wood", 50)]);

    assert_eq!(
        price,
        Price::from([("gold".to_string(), 100), ("wood".to_string(), 50)])
    );
}

#[test]
fn from_without_entries_is_empty() {
    let price = price::from(Vec::<(String, u32)>::new());

    assert!(price.is_empty());
}

#[test]
fn from_keeps_last_amount_for_repeated_kind() {
    let price = price::from([("gold", 100), ("gold", 25)]);

    assert_eq!(price, Price::from([("gold".to_string(), 25)]));
}
