use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages every player's timed buffs by one tick.
pub fn process_player_buffs(world: &mut World) {
    game_loop::stats::process_player_buffs(world);
}
