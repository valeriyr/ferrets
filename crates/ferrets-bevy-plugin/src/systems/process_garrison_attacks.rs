use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Lets garrisoned passengers fight from inside their holder (see
/// [`game_loop::garrison::process_garrison_attacks`]).
pub fn process_garrison_attacks(world: &mut World) {
    game_loop::garrison::process_garrison_attacks(world);
}
