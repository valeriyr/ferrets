//! Per-player skill cooldowns.
//!
//! The work of a cast lives in the buff it applies (see
//! [`PlayerBuffs`](crate::player_buffs::PlayerBuffs)); what remains here is only
//! when each player may cast each skill again. Mutations come from the
//! deterministic command path and the tick that ages them.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::{
    cooldown::Cooldown,
    events::{EventRecord, SimulationEvent},
    session::player_id::PlayerId,
};
use ferrets_content::skills::SkillId;

/// Skill cooldowns for all players in the session, indexed by [`PlayerId`].
#[derive(Resource, Debug)]
pub struct PlayerSkills {
    /// Where each skill's cooldown stands, per player; a skill not listed is
    /// ready.
    cooldowns: Vec<BTreeMap<SkillId, Cooldown>>,
}

impl PlayerSkills {
    /// Creates empty skill state for each player.
    pub fn new(player_count: usize) -> Self {
        Self {
            cooldowns: vec![BTreeMap::new(); player_count],
        }
    }

    /// Whether the player's `skill` is ready.
    pub fn ready(&self, player: PlayerId, skill: SkillId) -> bool {
        self.cooldown(player, skill) == Cooldown::Ready
    }

    /// Where the player's `skill` cooldown stands.
    pub fn cooldown(&self, player: PlayerId, skill: SkillId) -> Cooldown {
        self.cooldowns[player as usize]
            .get(&skill)
            .copied()
            .unwrap_or(Cooldown::Ready)
    }

    /// Starts the skill's cooldown after a cast.
    pub(crate) fn start_cooldown(&mut self, player: PlayerId, skill: SkillId, cooldown: u32) {
        self.cooldowns[player as usize].insert(skill, Cooldown::of(cooldown));
    }

    /// Ages every cooldown by one tick, dropping those that come ready.
    pub(crate) fn tick_cooldowns(&mut self) {
        for cooldowns in &mut self.cooldowns {
            for cooldown in cooldowns.values_mut() {
                *cooldown = cooldown.ticked();
            }
            cooldowns.retain(|_, cooldown| *cooldown != Cooldown::Ready);
        }
    }
}

/// Puts `skill` on cooldown for `player` and announces the cast.
///
/// The announcing counterpart to [`PlayerSkills::start_cooldown`], which starts
/// the timer and says nothing.
pub fn cast(world: &mut World, player: PlayerId, skill: SkillId, cooldown: u32) {
    world
        .resource_mut::<PlayerSkills>()
        .start_cooldown(player, skill, cooldown);
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::PlayerSkillCast { player, skill });
}
