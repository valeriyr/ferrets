use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Lines up each pool of every rising entity with its work, and settles the
/// pools of each entity risen since the last fold.
pub fn rise_sites(world: &mut World) {
    game_loop::build::rise_sites(world);
}
