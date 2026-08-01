use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Recomputes each player's effective stats from applied and entity-granted
/// modifiers.
pub fn recompute_player_stats(world: &mut World) {
    game_loop::stats::recompute_player_stats(world);
}
