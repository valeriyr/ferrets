//! Per-entity skill state: which abilities an entity has and their cooldowns.

use bevy_ecs::prelude::*;

use ferrets_content::skills::SkillId;

use crate::{
    command::SkillTarget,
    cooldown::Cooldown,
    entity_def,
    events::{EventRecord, SimulationEvent},
};

/// An entity's skills, each with where its cooldown stands.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct SkillsComponent {
    skills: Vec<(SkillId, Cooldown)>,
}

impl SkillsComponent {
    /// Creates a skills component from the given skill ids, all ready.
    pub fn new(skills: impl IntoIterator<Item = SkillId>) -> Self {
        Self {
            skills: skills.into_iter().map(|id| (id, Cooldown::Ready)).collect(),
        }
    }

    /// The skill ids this entity has, in declaration order.
    pub fn skills(&self) -> impl Iterator<Item = SkillId> + '_ {
        self.skills.iter().map(|&(id, _)| id)
    }

    /// `true` if the entity has skill `id` and it is ready.
    pub fn ready(&self, id: SkillId) -> bool {
        self.skills
            .iter()
            .any(|&(skill, cooldown)| skill == id && cooldown == Cooldown::Ready)
    }

    /// Where skill `id`'s cooldown stands.
    ///
    /// Panics if the entity lacks the skill: only a skill it has is asked
    /// after.
    pub fn cooldown(&self, id: SkillId) -> Cooldown {
        self.skills
            .iter()
            .find(|&&(skill, _)| skill == id)
            .map(|&(_, cooldown)| cooldown)
            .expect("only a skill the entity has is asked after")
    }

    /// Puts skill `id` on a cooldown `ticks` long.
    ///
    /// Panics if the entity lacks the skill: only a skill it has is cast.
    pub fn start_cooldown(&mut self, id: SkillId, ticks: u32) {
        *self.cooldown_mut(id) = Cooldown::of(ticks);
    }

    /// Makes skill `id` ready again at once.
    ///
    /// Panics if the entity lacks the skill: only a skill it has is cast.
    pub fn reset_cooldown(&mut self, id: SkillId) {
        *self.cooldown_mut(id) = Cooldown::Ready;
    }

    /// Ages every cooldown by one tick.
    pub fn tick_cooldowns(&mut self) {
        for (_, cooldown) in &mut self.skills {
            *cooldown = cooldown.ticked();
        }
    }

    /// Skill `id`'s cooldown, to rewrite. Panics if the entity lacks the
    /// skill.
    fn cooldown_mut(&mut self, id: SkillId) -> &mut Cooldown {
        self.skills
            .iter_mut()
            .find(|(skill, _)| *skill == id)
            .map(|(_, cooldown)| cooldown)
            .expect("only a skill the entity has is cast")
    }
}

/// Puts `caster`'s `skill` on a cooldown `ticks` long, announcing nothing.
///
/// Panics if `caster` has no skills: only an entity with skills casts.
pub fn start_cooldown(world: &mut World, caster: Entity, skill: SkillId, ticks: u32) {
    skills_mut(world, caster).start_cooldown(skill, ticks);
}

/// Makes `caster`'s `skill` ready again at once, announcing nothing.
///
/// Panics if `caster` has no skills: only an entity with skills casts.
pub fn reset_cooldown(world: &mut World, caster: Entity, skill: SkillId) {
    skills_mut(world, caster).reset_cooldown(skill);
}

/// Announces `caster`'s cast of `skill` landing against `target`.
pub fn announce_cast(world: &mut World, caster: Entity, target: SkillTarget, skill: SkillId) {
    let announced = SimulationEvent::SkillCast {
        caster: entity_def::simulation_id(world, caster),
        target,
        skill,
    };
    world.resource_mut::<EventRecord>().emit(announced);
}

/// `caster`'s skills, to rewrite. Panics if it has none.
fn skills_mut(world: &mut World, caster: Entity) -> Mut<'_, SkillsComponent> {
    world
        .get_mut::<SkillsComponent>(caster)
        .expect("only an entity with skills casts")
}
