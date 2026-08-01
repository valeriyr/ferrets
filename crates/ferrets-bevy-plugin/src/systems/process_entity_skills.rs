use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages every entity-skill cooldown by one tick.
pub fn process_entity_skills(world: &mut World) {
    game_loop::stats::process_entity_skills(world);
}
