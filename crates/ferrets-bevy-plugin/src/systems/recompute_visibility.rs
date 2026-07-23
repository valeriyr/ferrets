use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Recomputes per-player fog of war from the sight of owned entities.
pub fn recompute_visibility(world: &mut World) {
    game_loop::visibility::recompute_visibility(world);
}
