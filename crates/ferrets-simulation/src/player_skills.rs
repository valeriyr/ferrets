//! Per-player skill cooldowns.
//!
//! The work of a cast lives in the buff it applies (see
//! [`PlayerBuffs`](crate::player_buffs::PlayerBuffs)); what remains here is only
//! when each player may cast each skill again. Mutations come from the
//! deterministic command path and the tick that ages them.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::{content::skills::SkillId, session::player_slot::PlayerId};

/// Skill cooldowns for all players in the session, indexed by [`PlayerId`].
#[derive(Resource, Debug, Default)]
pub struct PlayerSkills {
    /// Remaining cooldown per skill, per player. Absent means ready.
    cooldowns: Vec<BTreeMap<SkillId, u32>>,
}

impl PlayerSkills {
    /// Creates empty skill state for each player.
    pub fn new(player_count: usize) -> Self {
        Self {
            cooldowns: vec![BTreeMap::new(); player_count],
        }
    }

    /// Whether the player's `skill` is off cooldown.
    pub fn ready(&self, player: PlayerId, skill: SkillId) -> bool {
        !self.cooldowns[player as usize].contains_key(&skill)
    }

    /// Remaining cooldown ticks for the player's `skill`; zero when ready.
    pub fn cooldown_remaining(&self, player: PlayerId, skill: SkillId) -> u32 {
        self.cooldowns[player as usize]
            .get(&skill)
            .copied()
            .unwrap_or(0)
    }

    /// Starts the skill's cooldown after a cast.
    pub(crate) fn start_cooldown(&mut self, player: PlayerId, skill: SkillId, cooldown: u32) {
        if cooldown > 0 {
            self.cooldowns[player as usize].insert(skill, cooldown);
        }
    }

    /// Ages every cooldown by one tick, dropping those that reach zero.
    pub(crate) fn tick_cooldowns(&mut self) {
        for cooldowns in &mut self.cooldowns {
            cooldowns.retain(|_, remaining| {
                *remaining -= 1;
                *remaining > 0
            });
        }
    }
}
