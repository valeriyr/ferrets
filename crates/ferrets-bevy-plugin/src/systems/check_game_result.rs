use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Applies the session's finish policy at the end of the tick, ending the game
/// once it is decided.
pub fn check_game_result(world: &mut World) {
    game_loop::game_result::check(world);
}
