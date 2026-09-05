use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Advances every construction site no builder works by one tick.
pub fn advance_sites_without_builder(world: &mut World) {
    game_loop::build::advance_sites_without_builder(world);
}
