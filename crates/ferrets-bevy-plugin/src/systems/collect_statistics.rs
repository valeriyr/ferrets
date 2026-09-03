use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Folds everything the completed tick announced into the per-player tallies.
pub fn collect_statistics(world: &mut World) {
    game_loop::tally::collect(world);
}
