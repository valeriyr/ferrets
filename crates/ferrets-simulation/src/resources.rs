//! Per-player resource stockpiles (gold, wood, …). Resource kinds are
//! content-defined strings, not hard-coded in the engine.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::session::player_slot::PlayerId;

/// A price in one or more resource kinds, e.g. `{"gold": 100, "wood": 50}`.
pub type Cost = BTreeMap<String, u32>;

/// One player's starting amount of one resource, as declared data seeding the
/// live stockpile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartingStock {
    /// The player whose stockpile is seeded.
    pub player: PlayerId,
    /// The resource to seed, by content name.
    pub resource: String,
    /// The amount the stockpile starts with.
    pub amount: u32,
}

/// Builds a [`Cost`] from `(kind, amount)` entries, converting keys to owned
/// strings. Does not validate amounts or kinds — the caller decides what counts
/// as valid.
pub fn cost(entries: impl IntoIterator<Item = (impl Into<String>, u32)>) -> Cost {
    entries
        .into_iter()
        .map(|(kind, amount)| (kind.into(), amount))
        .collect()
}

/// Resource stockpiles for all players in the session, indexed by [`PlayerId`].
#[derive(Resource)]
pub struct PlayerResources(Vec<BTreeMap<String, u32>>);

impl PlayerResources {
    /// Creates an empty stockpile for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![BTreeMap::new(); player_count])
    }

    /// Returns the amount of `kind` the player currently has.
    pub fn amount(&self, player: PlayerId, kind: &str) -> u32 {
        self.0[player as usize].get(kind).copied().unwrap_or(0)
    }

    /// Iterates every `(player, kind, amount)` in deterministic order — ascending
    /// player, then kind.
    pub fn iter(&self) -> impl Iterator<Item = (PlayerId, &str, u32)> {
        self.0.iter().enumerate().flat_map(|(player, stock)| {
            stock
                .iter()
                .map(move |(kind, &amount)| (player as PlayerId, kind.as_str(), amount))
        })
    }

    /// Adds `amount` of `kind` to the player's stockpile, saturating at
    /// [`u32::MAX`].
    pub fn add(&mut self, player: PlayerId, kind: &str, amount: u32) {
        let stock = self.0[player as usize].entry(kind.to_string()).or_insert(0);
        *stock = stock.saturating_add(amount);
    }

    /// Returns `true` if the player can pay `cost`.
    pub fn can_afford(&self, player: PlayerId, cost: &Cost) -> bool {
        cost.iter()
            .all(|(kind, amount)| self.amount(player, kind) >= *amount)
    }

    /// Subtracts `cost` from the player's stockpile.
    ///
    /// Panics if the player cannot afford it — check with [`Self::can_afford`] first.
    pub fn subtract(&mut self, player: PlayerId, cost: &Cost) {
        assert!(
            self.can_afford(player, cost),
            "player {player} cannot afford {cost:?}"
        );
        for (kind, amount) in cost {
            *self.0[player as usize].get_mut(kind).unwrap() -= amount;
        }
    }

    /// Adds `cost` back to the player's stockpile (e.g. a cancelled order refund).
    pub fn refund(&mut self, player: PlayerId, cost: &Cost) {
        for (kind, amount) in cost {
            self.add(player, kind, *amount);
        }
    }
}
