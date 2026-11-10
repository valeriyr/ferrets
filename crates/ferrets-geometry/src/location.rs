//! Where a body of a size stands on the map: its middle, the cell it is
//! anchored at, its footprint and its bounds.

use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use crate::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

/// The place of a body that lies wholly on the map: on each axis its middle
/// lies at least half its size from either edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// The body's middle.
    middle: FixedUVec2,
    /// The cells the body spans on each axis.
    size: CellSize,
}

impl Location {
    /// The place of a body of `size` with its middle at `middle`, on a map
    /// `map_size` cells across.
    ///
    /// Panics if the body would not lie wholly on the map.
    #[inline]
    pub fn new(middle: FixedUVec2, size: CellSize, map_size: CellSize) -> Self {
        Self::checked(middle, size, map_size).unwrap_or_else(|| {
            panic!("a body lies wholly on the map: {size:?} at {middle:?} on {map_size:?}")
        })
    }

    /// The place of a body of `size` with its middle at `middle`, on a map
    /// `map_size` cells across, or `None` when the body would not lie wholly on
    /// the map.
    #[inline]
    pub fn checked(middle: FixedUVec2, size: CellSize, map_size: CellSize) -> Option<Self> {
        let within = |middle: FixedU64, cells: u32, edge: u32| {
            middle >= half(cells) && middle + half(cells) <= FixedU64::from_num(edge)
        };
        (within(middle.x, size.width, map_size.width)
            && within(middle.y, size.height, map_size.height))
        .then_some(Self { middle, size })
    }

    /// The place nearest `middle` where a body of `size` lies wholly on a map
    /// `map_size` cells across: `middle` moved on each axis only as far as it
    /// needs to lie at least half the body's size from either edge.
    ///
    /// Panics if the body is wider or taller than the map.
    #[inline]
    pub fn clamped(middle: FixedUVec2, size: CellSize, map_size: CellSize) -> Self {
        assert!(
            size.fits_within(map_size),
            "a body fits on the map: {size:?} on {map_size:?}"
        );
        let clamp = |middle: FixedU64, cells: u32, edge: u32| {
            middle.clamp(half(cells), FixedU64::from_num(edge) - half(cells))
        };
        Self {
            middle: FixedUVec2::new(
                clamp(middle.x, size.width, map_size.width),
                clamp(middle.y, size.height, map_size.height),
            ),
            size,
        }
    }

    /// The place of a body of `size` anchored at `anchor`, on a map
    /// `map_size` cells across: its middle that of its footprint.
    ///
    /// Panics if the footprint would not lie wholly on the map.
    #[inline]
    pub fn at(anchor: CellPos, size: CellSize, map_size: CellSize) -> Self {
        Self::new(anchored_middle(anchor, size), size, map_size)
    }

    /// The place nearest that of a body of `size` anchored at `anchor` which
    /// lies wholly on a map `map_size` cells across, lined up on its cells.
    ///
    /// Panics if the body is wider or taller than the map.
    #[inline]
    pub fn at_clamped(anchor: CellPos, size: CellSize, map_size: CellSize) -> Self {
        Self::clamped(anchored_middle(anchor, size), size, map_size)
    }

    /// The body's middle.
    #[inline]
    pub fn middle(self) -> FixedUVec2 {
        self.middle
    }

    /// The cells the body spans on each axis.
    #[inline]
    pub fn size(self) -> CellSize {
        self.size
    }

    /// The cell the body is anchored at: the top-left cell of its footprint,
    /// its bounds' low corner rounded to the nearest cell, a half rounding up.
    #[inline]
    pub fn anchor(self) -> CellPos {
        let rounded = |middle: FixedU64, cells: u32| middle + FixedU64::ONE / 2 - half(cells);
        CellPos::from(FixedUVec2::new(
            rounded(self.middle.x, self.size.width),
            rounded(self.middle.y, self.size.height),
        ))
    }

    /// The cells the body is counted on: `size` cells from its anchor.
    #[inline]
    pub fn footprint(self) -> CellRect {
        CellRect::new(self.anchor(), self.size)
    }

    /// The body's exact extent: half its size either side of its middle.
    #[inline]
    pub fn bounds(self) -> FixedURect {
        let (width, height) = (half(self.size.width), half(self.size.height));
        FixedURect::new(
            FixedUVec2::new(self.middle.x - width, self.middle.y - height),
            FixedUVec2::new(self.middle.x + width, self.middle.y + height),
        )
    }

    /// The place snapped onto the cells the body is counted on: its middle that
    /// of its footprint.
    #[inline]
    pub fn snapped(self) -> Self {
        Self {
            middle: anchored_middle(self.anchor(), self.size),
            size: self.size,
        }
    }

    /// The place snapped onto the cells the body is counted on, a low edge
    /// halfway between two cells taking the one nearer `toward` on that axis
    /// rather than the one above.
    #[inline]
    pub fn snapped_toward(self, toward: CellPos) -> Self {
        let pick = |middle: FixedU64, cells: u32, toward: u32| {
            let low = middle - half(cells);
            let below = low.floor().to_num::<u32>();
            if low.frac() == FixedU64::ONE / 2 && toward <= below {
                below
            } else {
                (low + FixedU64::ONE / 2).floor().to_num::<u32>()
            }
        };
        let anchor = CellPos::new(
            pick(self.middle.x, self.size.width, toward.x),
            pick(self.middle.y, self.size.height, toward.y),
        );
        Self {
            middle: anchored_middle(anchor, self.size),
            size: self.size,
        }
    }

    /// Whether the body lies part way between its anchor's cells and
    /// another's, not lined up on its cells.
    #[inline]
    pub fn is_mid_crossing(self) -> bool {
        self != self.snapped()
    }
}

/// The middle of a footprint of `size` anchored at `anchor`.
fn anchored_middle(anchor: CellPos, size: CellSize) -> FixedUVec2 {
    let corner = FixedUVec2::from(anchor);
    FixedUVec2::new(corner.x + half(size.width), corner.y + half(size.height))
}

/// Half of `cells`.
fn half(cells: u32) -> FixedU64 {
    FixedU64::from_num(cells) / 2
}
