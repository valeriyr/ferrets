use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ages timed buffs, expiring any that reached the end of their duration.
pub fn process_buffs(world: &mut World) {
    game_loop::stats::process_buffs(world);
}
