//! The per-player stat store.
//!
//! Player-scoped numeric values (the supply ceiling, …) live here as fixed-point
//! base values plus effective values after modifiers, computed by the same
//! index-transparent store the per-entity stats use — the typed ids are the only
//! thing keeping the two groups apart, which is their whole job.
//!
//! Modifiers arrive from two sides: applied ones, refolded on every mutation,
//! and presence-derived ones (the player's active buffs), replaced each tick.
//! Player modifiers fold here; applied entity modifiers reach the player's
//! units at the entity recompute.
//! Mutations are simulation state and must come from deterministic paths only —
//! commands, scenario hooks, game rules.

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use crate::{
    content::{
        player_stats::{self, PlayerStatId},
        stats::{EntityModifier, PlayerModifier, StatStore},
    },
    session::player_slot::PlayerId,
};

/// One player's stats and the modifiers currently applied to them.
#[derive(Debug, Clone, Default)]
struct Store {
    /// The player's stat cells.
    stats: StatStore,
    /// Applied modifiers over the player's own stats, in application order.
    /// The fold is order-independent, so the order carries no meaning beyond
    /// removal bookkeeping.
    player_modifiers: Vec<PlayerModifier>,
    /// Applied modifiers laid over every unit the player owns — read by the
    /// entity recompute, never folded here.
    entity_modifiers: Vec<EntityModifier>,
    /// Modifiers the player's active buffs grant to its own stats, recomputed
    /// each tick, never mutated directly.
    derived: Vec<PlayerModifier>,
}

impl Store {
    /// Refolds every present stat's effective value under the applied and
    /// derived player modifiers together.
    fn refold(&mut self) {
        let targeting: Vec<_> = self
            .derived
            .iter()
            .chain(self.player_modifiers.iter())
            .map(|m| (m.stat.index(), m.op, m.magnitude))
            .collect();
        self.stats.recompute(&targeting, player_stats::floor_of);
    }
}

/// Stat stores for all players in the session, indexed by [`PlayerId`].
///
/// A stat a player was never given reads as `None`, which consumers interpret
/// per stat — an absent supply ceiling, for instance, means uncapped.
#[derive(Resource, Debug)]
pub struct PlayerStats(Vec<Store>);

impl PlayerStats {
    /// Creates an empty store for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![Store::default(); player_count])
    }

    /// Sets a player's base value for `stat`, growing the store as needed, and
    /// refolds its effective value under the player's current modifiers.
    pub fn set_base(&mut self, player: PlayerId, stat: PlayerStatId, base: FixedU64) {
        let store = &mut self.0[player as usize];
        store.stats.set_base(stat.index(), base);
        store.refold();
    }

    /// The base value of `stat`, or `None` if the player does not have it.
    pub fn base(&self, player: PlayerId, stat: PlayerStatId) -> Option<FixedU64> {
        self.0[player as usize].stats.base(stat.index())
    }

    /// The effective value of `stat` after modifiers, or `None` if the player
    /// does not have it.
    pub fn effective(&self, player: PlayerId, stat: PlayerStatId) -> Option<FixedU64> {
        self.0[player as usize].stats.effective(stat.index())
    }

    /// The effective value of `stat` truncated to a whole number, or `None` if
    /// the player does not have it — for integer-consuming callers.
    pub fn effective_as_u32(&self, player: PlayerId, stat: PlayerStatId) -> Option<u32> {
        self.effective(player, stat)
            .map(|value| value.to_num::<u32>())
    }

    /// Applies a modifier over the player's own stats and refolds them.
    ///
    /// A modifier over a stat the player does not have is kept but folds into
    /// nothing until the stat gains a base value.
    pub fn add_player_modifier(&mut self, player: PlayerId, modifier: PlayerModifier) {
        let store = &mut self.0[player as usize];
        store.player_modifiers.push(modifier);
        store.refold();
    }

    /// Removes one instance of an identical player modifier, if applied, and
    /// refolds.
    pub fn remove_player_modifier(&mut self, player: PlayerId, modifier: PlayerModifier) {
        let store = &mut self.0[player as usize];
        if let Some(position) = store.player_modifiers.iter().position(|m| *m == modifier) {
            store.player_modifiers.remove(position);
            store.refold();
        }
    }

    /// Applies a modifier over every unit the player owns. It takes effect at
    /// the next entity recompute.
    pub fn add_entity_modifier(&mut self, player: PlayerId, modifier: EntityModifier) {
        self.0[player as usize].entity_modifiers.push(modifier);
    }

    /// Removes one instance of an identical entity modifier, if applied.
    pub fn remove_entity_modifier(&mut self, player: PlayerId, modifier: EntityModifier) {
        let store = &mut self.0[player as usize];
        if let Some(position) = store.entity_modifiers.iter().position(|m| *m == modifier) {
            store.entity_modifiers.remove(position);
        }
    }

    /// Replaces the presence-derived contributions and refolds when they
    /// changed.
    ///
    /// Called once per tick with what the player's active buffs grant right
    /// now; a tick that grants the same set costs no refold.
    pub(crate) fn set_derived(&mut self, player: PlayerId, derived: Vec<PlayerModifier>) {
        let store = &mut self.0[player as usize];
        if store.derived != derived {
            store.derived = derived;
            store.refold();
        }
    }

    /// The applied modifiers currently laid over every unit the player owns.
    pub fn entity_modifiers(&self, player: PlayerId) -> &[EntityModifier] {
        &self.0[player as usize].entity_modifiers
    }
}
