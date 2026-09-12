use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Advances every breeder one tick: the lingering broodlings it takes in, the
/// births it owes, then the one its timer brings.
pub fn advance_brood(world: &mut World) {
    game_loop::brood::advance(world);
}
