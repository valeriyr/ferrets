//! Per-entity skill state: which abilities an entity has and their cooldowns.

use bevy_ecs::prelude::*;

use ferrets_content::skills::SkillId;

use crate::{
    entity_def,
    events::{EventRecord, SimulationEvent},
    simulation_id::SimulationId,
};

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

    /// Remaining cooldown ticks for skill `id`; zero when ready or absent.
    pub fn cooldown_remaining(&self, id: SkillId) -> u32 {
        self.skills
            .iter()
            .find(|&&(skill, _)| skill == id)
            .map(|&(_, remaining)| remaining)
            .unwrap_or(0)
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

/// Puts `skill` on `caster`'s cooldown and announces the cast against `target`.
///
/// The announcing counterpart to [`SkillsComponent::start_cooldown`], which
/// starts the timer and says nothing. A caster with no skills component is left
/// alone, and nothing is announced for a cast that could not have happened.
pub fn cast(
    world: &mut World,
    caster: Entity,
    target: SimulationId,
    skill: SkillId,
    cooldown: u32,
) {
    let mut caster_mut = world.entity_mut(caster);
    let Some(mut skills) = caster_mut.get_mut::<SkillsComponent>() else {
        return;
    };
    skills.start_cooldown(skill, cooldown);

    let announced = SimulationEvent::SkillCast {
        caster: entity_def::simulation_id(world, caster),
        target,
        skill,
    };
    world.resource_mut::<EventRecord>().emit(announced);
}
