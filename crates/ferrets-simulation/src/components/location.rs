//! World position and facing for simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::{facing::Facing, fixed_uvec2::FixedUVec2};

/// World position and facing of an entity in fixed-point grid units.
///
/// One unit = one grid cell.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LocationComponent {
    /// The body's middle, in simulation coordinates with sub-cell precision.
    ///
    /// Under the cell movement model a body keeps to the cell lattice: lined up
    /// on its cells, its middle is its anchored position, and mid-crossing it
    /// lies on the straight segment between the anchored positions of its
    /// crossing's two anchors — every write outside the movement step must
    /// preserve this. Under the continuous model a mover's position is any
    /// point that keeps its body on the map; entities that cannot move keep to
    /// anchored positions in both models.
    pub position: FixedUVec2,
    /// Which way the body itself points. A weapon that bears independently of the
    /// body keeps its own.
    pub facing: Facing,
}

impl LocationComponent {
    /// A location with its body's middle at `position`, facing `facing`.
    #[inline]
    pub fn new(position: FixedUVec2, facing: Facing) -> Self {
        Self { position, facing }
    }
}
