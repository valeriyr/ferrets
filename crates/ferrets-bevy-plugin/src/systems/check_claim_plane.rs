use bevy::prelude::*;
use ferrets_simulation::game_loop;

/// Checks, in debug builds, that the claim plane matches the bodies on it.
pub fn check_claim_plane(world: &mut World) {
    game_loop::claim_plane::check(world);
}
