use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Recomputes every field from the sources standing this tick.
pub fn recompute_fields(world: &mut World) {
    game_loop::fields::recompute_fields(world);
}
