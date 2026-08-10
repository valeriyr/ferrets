//! Active buffs on each player.
//!
//! A player-level buff is a registered
//! [`PlayerBuffDef`](ferrets_content::player_buffs::PlayerBuffDef) held by the player:
//! its player modifiers reach the player's own stats, and its entity modifiers
//! reach every unit the player owns.

use bevy_ecs::prelude::*;

use crate::{buffs_store::BuffsStore, session::player_slot::PlayerId};
use ferrets_content::{player_buffs::PlayerBuffId, stack_rule::StackRule};

/// The active buffs of all players in the session, indexed by [`PlayerId`] —
/// the player-level counterpart of the per-entity buffs component, sharing its
/// store.
#[derive(Resource, Debug, Default)]
pub struct PlayerBuffs(Vec<BuffsStore<PlayerBuffId>>);

impl PlayerBuffs {
    /// Creates an empty buff set for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![BuffsStore::default(); player_count])
    }

    /// Applies the buff `id` to `player` with the given lifetime, resolving
    /// stacking against any active instance of the same id per `stack_rule`.
    pub fn apply(
        &mut self,
        player: PlayerId,
        id: PlayerBuffId,
        stack_rule: StackRule,
        duration: Option<u32>,
    ) {
        self.0[player as usize].apply(id, stack_rule, duration);
    }

    /// Removes every active instance of `id` from `player`. Returns `true` if
    /// any was removed.
    pub fn remove(&mut self, player: PlayerId, id: PlayerBuffId) -> bool {
        self.0[player as usize].remove(id)
    }

    /// The player's active buffs as `(id, stacks)` pairs.
    pub fn active(&self, player: PlayerId) -> impl Iterator<Item = (PlayerBuffId, u32)> + '_ {
        self.0[player as usize].active()
    }

    /// Ages every player's timed buffs by one tick, dropping any that expire.
    pub(crate) fn tick_down(&mut self) {
        for buffs in &mut self.0 {
            buffs.tick_down();
        }
    }
}
