use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Advances every unattended rising entity by one tick.
pub fn advance_unattended(world: &mut World) {
    game_loop::rise::advance_unattended(world);
}
