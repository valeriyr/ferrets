use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Ends each rise bound to field rules whose footprint has left them.
pub fn lose_rises_outside_fields(world: &mut World) {
    game_loop::rise::lose_outside_fields(world);
}
