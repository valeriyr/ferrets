use bevy::prelude::*;
use ferrets_simulation::berths;

/// Moves every attached worker one tick along its stance: between the berths
/// of its group, or round the one it holds.
pub fn advance_berths(world: &mut World) {
    berths::advance(world);
}
