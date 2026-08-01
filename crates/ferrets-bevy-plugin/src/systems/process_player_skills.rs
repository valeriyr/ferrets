use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages player-skill cooldowns and casts, expiring finished casts.
pub fn process_player_skills(world: &mut World) {
    game_loop::stats::process_player_skills(world);
}
