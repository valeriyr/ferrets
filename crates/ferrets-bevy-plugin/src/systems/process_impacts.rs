use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Lands every shot whose flight time has elapsed.
pub fn process_impacts(world: &mut World) {
    game_loop::impacts::process_impacts(world);
}
