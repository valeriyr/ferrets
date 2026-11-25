//! Content-defined location property struct and its footprint solidity.

use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::domain::DomainId;

/// Whether an entity's footprint blocks the cells it stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solidity {
    /// The footprint claims the entity's layers: others collide with it, and
    /// no footprint is raised over a body that never walks, no solid one over
    /// a mover.
    Solid,
    /// The footprint claims nothing, so movers cross it freely, but the
    /// entity is still there: no static footprint is raised over a body that
    /// never walks, no solid static one over a mover.
    Underfoot,
    /// The footprint claims nothing, so movers cross it, and entities of this
    /// solidity share its cells, but no static footprint of another solidity
    /// is raised over a body that never walks, no solid static one over a
    /// mover.
    Passable,
}

impl Solidity {
    /// Whether a footprint of this solidity marks the cells it covers occupied.
    #[inline]
    pub fn claims_cells(self) -> bool {
        match self {
            Solidity::Solid => true,
            Solidity::Underfoot | Solidity::Passable => false,
        }
    }

    /// Whether a footprint of this solidity keeps a static footprint of
    /// `raised` from being raised over the cells it covers.
    #[inline]
    pub fn keeps_off(self, raised: Solidity) -> bool {
        match (self, raised) {
            (
                Solidity::Solid | Solidity::Underfoot,
                Solidity::Solid | Solidity::Underfoot | Solidity::Passable,
            ) => true,
            (Solidity::Passable, Solidity::Solid | Solidity::Underfoot) => true,
            (Solidity::Passable, Solidity::Passable) => false,
        }
    }
}

/// Content-defined location properties for an entity type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationDef {
    /// The domain: the one layer the body moves in, and the terrains it may be
    /// over there.
    domain: DomainId,
    /// Footprint in whole grid cells.
    size: CellSize,
    /// The radius of the body's circle in cells, where the continuous movement
    /// model resolves contact; `None` for a body with no circle.
    radius: Option<FixedU64>,
    /// Whether the footprint also claims the occupied layers.
    solidity: Solidity,
    /// The layers whose cells the footprint takes, and in which it blocks and
    /// collides: its domain's layer, and any further layers it towers into.
    occupies: LayerMask,
}

/// The largest radius a body's circle may have, in cells.
pub const MAX_RADIUS: FixedU64 = FixedU64::const_from_int(1024);

impl LocationDef {
    /// Creates a new `LocationDef` with the given data.
    ///
    /// Panics if `occupies` is empty, `size` has a zero dimension, or a
    /// declared `radius` is zero or above [`MAX_RADIUS`].
    #[inline]
    pub fn new(
        domain: DomainId,
        size: CellSize,
        radius: Option<FixedU64>,
        solidity: Solidity,
        occupies: impl Into<LayerMask>,
    ) -> Self {
        let occupies = occupies.into();

        assert!(occupies != LayerMask::EMPTY, "occupies must not be empty");
        assert!(
            size.width > 0 && size.height > 0,
            "size dimensions must be greater than 0"
        );
        assert!(
            radius.is_none_or(|radius| radius > FixedU64::ZERO),
            "radius must be greater than 0"
        );
        assert!(
            radius.is_none_or(|radius| radius <= MAX_RADIUS),
            "radius must be at most {MAX_RADIUS}"
        );

        Self {
            domain,
            size,
            radius,
            solidity,
            occupies,
        }
    }

    /// Returns the domain: the one layer the body moves in, and the terrains
    /// it may be over there.
    #[inline]
    pub fn domain(&self) -> DomainId {
        self.domain
    }

    /// Returns the layers whose cells the footprint takes, and in which it
    /// blocks and collides: its domain's layer, and any further layers it
    /// towers into.
    #[inline]
    pub fn occupies(&self) -> LayerMask {
        self.occupies
    }

    /// Returns whether the footprint claims the entity's layers.
    #[inline]
    pub fn solidity(&self) -> Solidity {
        self.solidity
    }

    /// Returns the entity's footprint size in grid cells.
    #[inline]
    pub fn size(&self) -> CellSize {
        self.size
    }

    /// Returns the radius of the body's circle in cells, or `None` for a body
    /// with no circle.
    #[inline]
    pub fn radius(&self) -> Option<FixedU64> {
        self.radius
    }
}
