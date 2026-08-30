//! World position and facing for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::{facing::Facing, fixed_uvec2::FixedUVec2};

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
    /// Which way the body itself points. A weapon that bears independently of the
    /// body keeps its own.
    pub facing: Facing,
}

impl LocationComponent {
    /// Creates a new `LocationComponent` with the given data.
    #[inline]
    pub fn new(position: FixedUVec2, facing: Facing) -> Self {
        Self { position, facing }
    }
}
