use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Refits the concealment marker from what conceals each entity this tick.
pub fn refit_concealment(world: &mut World) {
    game_loop::conceal::refit_concealment(world);
}
