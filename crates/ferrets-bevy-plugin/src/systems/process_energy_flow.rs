use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Moves each energy pool by one tick's regeneration and drain.
pub fn process_energy_flow(world: &mut World) {
    game_loop::flows::process_energy_flow(world);
}
