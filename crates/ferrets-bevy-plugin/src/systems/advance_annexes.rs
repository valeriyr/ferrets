use bevy::prelude::*;
use ferrets_simulation::annex;

/// Re-derives which primary every annex stands with, applies what standing
/// with no primary does to it, and hands over the annexes a claim gives away.
pub fn advance_annexes(world: &mut World) {
    annex::advance(world);
}
