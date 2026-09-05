use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Performs the one-off acts of every entity that has just come to stand.
pub fn perform_standing_acts(world: &mut World) {
    game_loop::stand::perform_standing_acts(world);
}
