use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages every skill cooldown by one tick.
pub fn process_cooldowns(world: &mut World) {
    game_loop::stats::process_cooldowns(world);
}
