//! Skills: content-defined activated abilities — an effect on a target, gated by
//! a cooldown and an energy cost.

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;
use serde::{Deserialize, Serialize};

use super::buffs::BuffId;

/// A handle to a registered skill, assigned in registration order.
///
/// Content declares skills by name and the registry mints their ids, so identical
/// content registered in the same order resolves to identical ids on every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SkillId(u16);

impl SkillId {
    /// Creates a skill id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more skills registered than SkillId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Who a skill acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillTarget {
    /// The caster itself; the use command carries no target.
    Caster,
    /// An owned or allied entity.
    Ally,
    /// A hostile entity.
    Enemy,
}

/// What using a skill does to its resolved target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillEffect {
    /// Applies the buff with the given id.
    ApplyBuff(BuffId),
    /// Removes every active buff of the given id.
    RemoveBuff(BuffId),
    /// Deals flat damage, bypassing armor (like an ability, not a weapon).
    Damage(FixedU64),
    /// Restores health, up to the target's maximum.
    Heal(FixedU64),
}

/// A content-defined skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDef {
    /// Ticks before the skill can be used again.
    pub cooldown: u32,
    /// Energy spent per use.
    pub energy_cost: FixedU64,
    /// Who the skill acts on.
    pub target: SkillTarget,
    /// What the skill does.
    pub effect: SkillEffect,
}

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
