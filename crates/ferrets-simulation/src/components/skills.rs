//! Per-entity skill state: which abilities an entity has and their cooldowns.

use bevy_ecs::prelude::*;

use crate::content::skills::SkillId;

/// An entity's skills, each paired with the ticks remaining on its cooldown.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsComponent {
    skills: Vec<(SkillId, u32)>,
}

impl SkillsComponent {
    /// Creates a skills component from the given skill ids, all off cooldown.
    pub fn new(skills: impl IntoIterator<Item = SkillId>) -> Self {
        Self {
            skills: skills.into_iter().map(|id| (id, 0)).collect(),
        }
    }

    /// The skill ids this entity has, in declaration order.
    pub fn skills(&self) -> impl Iterator<Item = SkillId> + '_ {
        self.skills.iter().map(|&(id, _)| id)
    }

    /// `true` if the entity has skill `id` and it is off cooldown.
    pub fn ready(&self, id: SkillId) -> bool {
        self.skills
            .iter()
            .any(|&(skill, remaining)| skill == id && remaining == 0)
    }

    /// Puts skill `id` on `cooldown` ticks; a no-op if the entity lacks it.
    pub fn start_cooldown(&mut self, id: SkillId, cooldown: u32) {
        if let Some((_, remaining)) = self.skills.iter_mut().find(|(skill, _)| *skill == id) {
            *remaining = cooldown;
        }
    }

    /// Decrements every cooldown by one tick.
    pub fn tick_cooldowns(&mut self) {
        for (_, remaining) in &mut self.skills {
            *remaining = remaining.saturating_sub(1);
        }
    }
}
