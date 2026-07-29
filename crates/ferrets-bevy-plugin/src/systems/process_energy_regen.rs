use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Refills each energy pool by one tick's regeneration.
pub fn process_energy_regen(world: &mut World) {
    game_loop::stats::process_energy_regen(world);
}
