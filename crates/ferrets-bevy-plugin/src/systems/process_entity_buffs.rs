use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages every entity's timed buffs by one tick.
pub fn process_entity_buffs(world: &mut World) {
    game_loop::stats::process_entity_buffs(world);
}
