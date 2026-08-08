//! World position and facing for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};

/// World position and facing of an entity in fixed-point grid units.
///
/// One unit = one grid cell. Sub-unit precision supports smooth movement between
/// cells; integer values land on cell origin corners.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationComponent {
    /// Where the entity stands, in simulation coordinates.
    ///
    /// Under the cell movement model, positions keep to the cell lattice: an
    /// entity at rest stands exactly on a cell's origin corner, and a
    /// mid-crossing entity lies on the straight segment between the origin
    /// corners of its crossing's two cells — every write outside the
    /// movement step must preserve this. Under the continuous model a
    /// mover's position is any in-bounds point; entities that cannot move
    /// keep to cell origins in both models.
    pub position: FixedUVec2,
    /// The last look direction. The renderer normalizes it to the nearest sprite
    /// direction (8-way, 16-way, etc.).
    pub facing: FixedVec2,
}

impl LocationComponent {
    /// Creates a new `LocationComponent` with the given data.
    #[inline]
    pub fn new(position: FixedUVec2, facing: FixedVec2) -> Self {
        Self { position, facing }
    }
}
