//! Per-player resource stockpiles (gold, wood, …). Resource kinds are
//! content-defined strings, not hard-coded in the engine.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use ferrets_content::costs::Cost;
use serde::{Deserialize, Serialize};

use crate::{
    events::{EventRecord, SimulationEvent, SpendCause},
    session::player_id::PlayerId,
    simulation_id::SimulationId,
};

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

    /// Adds every arm of `cost` to the player's stockpile.
    ///
    /// The multi-kind form of [`Self::add`], and the inverse of
    /// [`Self::withdraw`]. Announces nothing; [`refund`] is the announcing
    /// counterpart.
    pub fn deposit(&mut self, player: PlayerId, cost: &Cost) {
        for (kind, amount) in cost {
            self.add(player, kind, *amount);
        }
    }

    /// Returns `true` if the player can pay `cost`.
    pub fn can_afford(&self, player: PlayerId, cost: &Cost) -> bool {
        cost.iter()
            .all(|(kind, amount)| self.amount(player, kind) >= *amount)
    }

    /// Subtracts every arm of `cost` from the player's stockpile.
    ///
    /// The inverse of [`Self::deposit`]. Announces nothing; [`charge`] is the
    /// announcing counterpart.
    ///
    /// Panics if the player cannot afford it — check with [`Self::can_afford`] first.
    pub fn withdraw(&mut self, player: PlayerId, cost: &Cost) {
        assert!(
            self.can_afford(player, cost),
            "player {player} cannot afford {cost:?}"
        );
        for (kind, amount) in cost {
            *self.0[player as usize].get_mut(kind).unwrap() -= amount;
        }
    }
}

/// Charges `cost` to `player` and announces what was spent. An empty `cost`
/// charges nothing and announces nothing.
///
/// Panics if the player cannot afford it — check with
/// [`PlayerResources::can_afford`] first.
pub fn charge(world: &mut World, player: PlayerId, cost: Cost, cause: SpendCause) {
    if cost.is_empty() {
        return;
    }
    world
        .resource_mut::<PlayerResources>()
        .withdraw(player, &cost);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ResourcesSpent {
            player,
            cost,
            cause,
        });
}

/// Gives `cost` back to `player` and announces the refund. An empty `cost`
/// returns nothing and announces nothing.
pub fn refund(world: &mut World, player: PlayerId, cost: Cost, cause: SpendCause) {
    if cost.is_empty() {
        return;
    }
    world
        .resource_mut::<PlayerResources>()
        .deposit(player, &cost);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ResourcesRefunded {
            player,
            cost,
            cause,
        });
}

/// Banks a carried load and announces the gather.
pub fn credit_gathered(
    world: &mut World,
    player: PlayerId,
    kind: &str,
    amount: u32,
    storage: SimulationId,
) {
    world
        .resource_mut::<PlayerResources>()
        .add(player, kind, amount);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ResourcesGathered {
            player,
            kind: kind.to_string(),
            amount,
            storage,
        });
}
