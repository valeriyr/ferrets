//! Grid-aligned rectangle of cells.

use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use crate::{cell_pos::CellPos, cell_size::CellSize};

/// A rectangle of whole cells: an origin and the size spanned from it — a
/// footprint as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRect {
    /// The rectangle's lowest-coordinate cell.
    pub origin: CellPos,
    /// How many cells the rectangle spans on each axis.
    pub size: CellSize,
}

impl CellRect {
    #[inline]
    pub const fn new(origin: CellPos, size: CellSize) -> Self {
        Self { origin, size }
    }

    /// The rectangle covering a single cell.
    #[inline]
    pub const fn cell(origin: CellPos) -> Self {
        Self::new(origin, CellSize::ONE)
    }

    /// The cells `bounds` lies over: every cell it shares area with, an edge
    /// on a cell boundary entering no cell beyond it. A side of no length
    /// lies over the one cell its coordinate falls in.
    #[inline]
    pub fn covering(bounds: FixedURect) -> Self {
        let span = |low: FixedU64, high: FixedU64| {
            let first = low.floor().to_num::<u32>();
            let last = high.ceil().to_num::<u32>().saturating_sub(1).max(first);
            (first, last - first + 1)
        };
        let (x, width) = span(bounds.min().x, bounds.max().x);
        let (y, height) = span(bounds.min().y, bounds.max().y);
        Self::new(CellPos::new(x, y), CellSize::new(width, height))
    }

    /// The exact rectangle the cells span, from the first cell's corner to
    /// the far corner of the last.
    #[inline]
    pub fn bounds(self) -> FixedURect {
        let corner = FixedUVec2::from(self.origin);
        FixedURect::new(
            corner,
            FixedUVec2::new(
                corner.x + FixedU64::from_num(self.size.width),
                corner.y + FixedU64::from_num(self.size.height),
            ),
        )
    }

    /// Returns `true` if the rectangle covers `pos`.
    #[inline]
    pub fn contains(self, pos: CellPos) -> bool {
        pos.x >= self.origin.x
            && pos.x < self.origin.x + self.size.width
            && pos.y >= self.origin.y
            && pos.y < self.origin.y + self.size.height
    }

    /// Returns `true` if the two rectangles share any cell.
    pub fn intersects(self, other: CellRect) -> bool {
        self.origin.x < other.origin.x + other.size.width
            && other.origin.x < self.origin.x + self.size.width
            && self.origin.y < other.origin.y + other.size.height
            && other.origin.y < self.origin.y + self.size.height
    }

    /// The cells the rectangle covers, in row-major order.
    pub fn cells(self) -> impl Iterator<Item = CellPos> {
        (0..self.size.height).flat_map(move |dy| {
            (0..self.size.width).map(move |dx| CellPos::new(self.origin.x + dx, self.origin.y + dy))
        })
    }

    /// The smallest rectangle covering both `self` and `other`.
    pub fn union(self, other: CellRect) -> CellRect {
        let origin = CellPos::new(
            self.origin.x.min(other.origin.x),
            self.origin.y.min(other.origin.y),
        );
        let end_x = (self.origin.x + self.size.width).max(other.origin.x + other.size.width);
        let end_y = (self.origin.y + self.size.height).max(other.origin.y + other.size.height);
        CellRect::new(origin, CellSize::new(end_x - origin.x, end_y - origin.y))
    }

    /// The cells of `self` outside `other`, as at most four disjoint
    /// rectangles: the rows below `other`'s, the columns left and right of it
    /// across its rows, and the rows above it. All of `self` when the two
    /// share no cell; nothing when `other` covers `self`.
    pub fn minus(self, other: CellRect) -> impl Iterator<Item = CellRect> {
        let (left, right) = (self.origin.x, self.origin.x + self.size.width);
        let (low, high) = (self.origin.y, self.origin.y + self.size.height);
        // The cut is `other` clamped into `self`: empty on an axis where the
        // two do not overlap, so the pieces around it then cover all of
        // `self`.
        let cut_left = other.origin.x.clamp(left, right);
        let cut_right = (other.origin.x + other.size.width).clamp(cut_left, right);
        let cut_low = other.origin.y.clamp(low, high);
        let cut_high = (other.origin.y + other.size.height).clamp(cut_low, high);
        [
            (left, low, right, cut_low),
            (left, cut_low, cut_left, cut_high),
            (cut_right, cut_low, right, cut_high),
            (left, cut_high, right, high),
        ]
        .into_iter()
        .filter(|&(x0, y0, x1, y1)| x1 > x0 && y1 > y0)
        .map(|(x0, y0, x1, y1)| {
            CellRect::new(CellPos::new(x0, y0), CellSize::new(x1 - x0, y1 - y0))
        })
    }

    /// The rect an anchor measures against when stopping within `distance`
    /// of `self` with a `size` footprint: grown low for a ranged stop, so
    /// the anchor measures the footprint's nearest edge; unchanged for a
    /// stop of zero, which is an anchor contract — the walk stands on the
    /// rect itself.
    pub fn accepted_by(self, size: CellSize, distance: u32) -> CellRect {
        if distance > 0 {
            self.grown_low(size)
        } else {
            self
        }
    }

    /// The rectangle grown by `size − 1` toward the low coordinates on both
    /// axes; the far edge never moves. Clamped at the grid origin, where the
    /// growth has nowhere to go.
    ///
    /// This makes a plain anchor measurement footprint-true: a `size`
    /// footprint is within some range of `self` exactly when its anchor is
    /// within that range of the grown rect, because a footprint extends from
    /// its anchor toward higher coordinates and the anchor trails it by up
    /// to `size − 1` on the low side only.
    ///
    /// Panics if `size` has a zero dimension, which no footprint has.
    pub fn grown_low(self, size: CellSize) -> CellRect {
        assert!(
            size.width > 0 && size.height > 0,
            "size dimensions must be greater than 0"
        );
        let origin = CellPos::new(
            self.origin.x.saturating_sub(size.width - 1),
            self.origin.y.saturating_sub(size.height - 1),
        );
        CellRect::new(
            origin,
            CellSize::new(
                self.origin.x + self.size.width - origin.x,
                self.origin.y + self.size.height - origin.y,
            ),
        )
    }
}
