use bevy::prelude::*;
use ferrets_simulation::watches::Watches;

/// Takes a tick off every watch and drops the ones that have run out, after
/// everything reading them this tick has run — so the next tick's fog reads
/// only what still holds.
pub fn age_watches(mut watches: ResMut<Watches>) {
    watches.tick_down();
}
