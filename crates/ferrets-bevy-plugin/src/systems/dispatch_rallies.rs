use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Gives every unit released this tick by a holder with a rally point set its
/// rally order.
pub fn dispatch_rallies(world: &mut World) {
    game_loop::rally::dispatch(world);
}
