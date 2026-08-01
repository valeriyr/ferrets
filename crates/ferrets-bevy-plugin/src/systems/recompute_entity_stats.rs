use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Folds active buffs into every buffed entity's effective stats for this tick.
pub fn recompute_entity_stats(world: &mut World) {
    game_loop::stats::recompute_entity_stats(world);
}
