//! Prices in content-defined resource kinds.

use std::collections::BTreeMap;

/// A price in one or more resource kinds, e.g. `{"gold": 100, "wood": 50}`.
pub type Price = BTreeMap<String, u32>;

/// Builds a [`Price`] from `(kind, amount)` entries, converting keys to owned
/// strings. Does not validate amounts or kinds — the caller decides what counts
/// as valid.
pub fn from(entries: impl IntoIterator<Item = (impl Into<String>, u32)>) -> Price {
    entries
        .into_iter()
        .map(|(kind, amount)| (kind.into(), amount))
        .collect()
}
