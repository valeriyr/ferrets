//! Skill cooldowns, aged each tick.

use bevy_ecs::world::World;

use crate::{
    components::entity_skills::SkillsComponent, entity_index::EntityIndex,
    player_skills::PlayerSkills,
};

/// Ages player-skill cooldowns by one tick. The buffs a cast applied age with
/// every other player buff in [`super::buffs::process_player_buffs`].
pub fn process_player_skills(world: &mut World) {
    world.resource_mut::<PlayerSkills>().tick_cooldowns();
}

/// Ages every entity-skill cooldown by one tick.
pub fn process_entity_skills(world: &mut World) {
    for (_, entity) in world.resource::<EntityIndex>().alive_entries() {
        if let Some(mut skills) = world.entity_mut(entity).get_mut::<SkillsComponent>() {
            skills.tick_cooldowns();
        }
    }
}
