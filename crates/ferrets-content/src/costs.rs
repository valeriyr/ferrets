//! Prices in content-defined resource kinds.

use std::collections::BTreeMap;

/// A price in one or more resource kinds, e.g. `{"gold": 100, "wood": 50}`.
pub type Cost = BTreeMap<String, u32>;

/// Builds a [`Cost`] from `(kind, amount)` entries, converting keys to owned
/// strings. Does not validate amounts or kinds — the caller decides what counts
/// as valid.
pub fn cost(entries: impl IntoIterator<Item = (impl Into<String>, u32)>) -> Cost {
    entries
        .into_iter()
        .map(|(kind, amount)| (kind.into(), amount))
        .collect()
}
