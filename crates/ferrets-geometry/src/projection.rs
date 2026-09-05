//! The projection: how the map measures distance, movement cost, and travel.

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use serde::{Deserialize, Serialize};

use crate::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

/// Movement cost for a cardinal (non-diagonal) step.
const CARDINAL_COST: u32 = 10;
/// Movement cost for a diagonal step — approximates √2 × [`CARDINAL_COST`].
const DIAGONAL_COST: u32 = 14;

/// Chebyshev distance — maximum of horizontal and vertical distances between
/// two positions.
pub fn chebyshev(a: CellPos, b: CellPos) -> u32 {
    a.x.abs_diff(b.x).max(a.y.abs_diff(b.y))
}

/// Octile distance — minimum movement cost accounting for diagonal and
/// cardinal step costs.
pub fn octile(a: CellPos, b: CellPos) -> u32 {
    let dx = a.x.abs_diff(b.x);
    let dy = a.y.abs_diff(b.y);

    let diag = dx.min(dy);
    let straight = dx.max(dy) - diag;

    diag * DIAGONAL_COST + straight * CARDINAL_COST
}

/// A grid step's direction class: along one axis, or along both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Cardinal,
    Diagonal,
}

/// Defines movement costs and range metrics for the map type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Projection {
    /// Isometric projection — all 8 directions cost equally and appear the same distance on screen.
    Isometric,
    /// Orthogonal top-down — diagonal moves cover √2 more ground and cost more.
    Orthogonal,
}

impl Projection {
    /// Movement cost for a single step.
    pub fn step_cost(self, step: Step) -> u32 {
        match self {
            Projection::Isometric => CARDINAL_COST,
            Projection::Orthogonal => match step {
                Step::Cardinal => CARDINAL_COST,
                Step::Diagonal => DIAGONAL_COST,
            },
        }
    }

    /// The admissible cost estimate between two cells under this projection's
    /// step costs.
    pub fn metric(self, from: CellPos, to: CellPos) -> u32 {
        match self {
            Projection::Isometric => chebyshev(from, to) * CARDINAL_COST,
            Projection::Orthogonal => octile(from, to),
        }
    }

    /// Returns `true` if `from` is within `distance` of `to`.
    ///
    /// The metric depends on the projection: Chebyshev cells for `Isometric`,
    /// Euclidean cells for `Orthogonal`.
    pub fn in_range(self, from: CellPos, to: CellPos, distance: u32) -> bool {
        self.gap_in_range(from.x.abs_diff(to.x), from.y.abs_diff(to.y), distance)
    }

    /// Returns `true` if `from` is within `distance` of `rect` (e.g. a
    /// building footprint).
    ///
    /// The distance is measured to the nearest cell of the rectangle, using
    /// the same metric as [`in_range`](Self::in_range). `from` is a single
    /// cell: measuring a wide mover's reach by its anchor overstates the
    /// distance — pass the goal through
    /// [`CellRect::grown_low`](crate::cell_rect::CellRect::grown_low)
    /// first, or use [`in_range_for_rects`](Self::in_range_for_rects).
    pub fn in_range_of_rect(self, from: CellPos, rect: CellRect, distance: u32) -> bool {
        let nearest = from.clamp_to_rect(rect);
        self.in_range(from, nearest, distance)
    }

    /// Every cell within `distance` of `rect` (see
    /// [`in_range_of_rect`](Self::in_range_of_rect)), row-major. Cells are
    /// clamped to non-negative coordinates and unbounded above.
    pub fn cells_in_range_of_rect(self, rect: CellRect, distance: u32) -> Vec<CellPos> {
        let min_x = rect.origin.x.saturating_sub(distance);
        let min_y = rect.origin.y.saturating_sub(distance);
        let max_x = rect.origin.x + rect.size.width + distance;
        let max_y = rect.origin.y + rect.size.height + distance;
        let mut cells = Vec::new();
        for y in min_y..max_y {
            for x in min_x..max_x {
                let cell = CellPos::new(x, y);
                if self.in_range_of_rect(cell, rect, distance) {
                    cells.push(cell);
                }
            }
        }
        cells
    }

    /// Whether a `size` footprint anchored at `from` satisfies stopping
    /// within `distance` of `goal`: measured by the footprint's nearest edge
    /// for a ranged stop, by the anchor itself for a stop of zero (see
    /// [`CellRect::accepted_by`]).
    pub fn in_reach(self, from: CellPos, size: CellSize, goal: CellRect, distance: u32) -> bool {
        self.in_range_of_rect(from, goal.accepted_by(size, distance), distance)
    }

    /// Whether any cell of `from` is within `distance` of any cell of `to`.
    ///
    /// Both sides are rectangles, so a wide mover reaches as far as its nearest
    /// edge does rather than as far as its anchor cell does — otherwise a
    /// two-cell body would have to walk a cell deeper than a one-cell body to
    /// count as adjacent to the same thing.
    pub fn in_range_for_rects(self, from: CellRect, to: CellRect, distance: u32) -> bool {
        let (dx, dy) = rect_gap(from, to);
        self.gap_in_range(dx, dy, distance)
    }

    /// Distance from `from` to the nearest cell of `rect`, using the same
    /// metric as [`in_range`](Self::in_range).
    ///
    /// Comparable only within a single projection: Chebyshev cells for
    /// `Isometric`, squared Euclidean cells for `Orthogonal` — so it is not
    /// an absolute cell count.
    pub fn rect_distance(self, from: CellPos, rect: CellRect) -> u32 {
        let nearest = from.clamp_to_rect(rect);
        match self {
            Projection::Isometric => chebyshev(from, nearest),
            Projection::Orthogonal => {
                // Squared in u64 and saturated, like `in_range`, so huge
                // maps rank correctly instead of overflowing; ranks past
                // the ceiling all tie as "immeasurably far".
                let dx = from.x.abs_diff(nearest.x) as u64;
                let dy = from.y.abs_diff(nearest.y) as u64;
                u32::try_from(dx * dx + dy * dy).unwrap_or(u32::MAX)
            }
        }
    }

    /// Distance between two rectangles, measured between their nearest
    /// cells — the rect-to-rect rank behind
    /// [`in_range_for_rects`](Self::in_range_for_rects), on the same scale
    /// as [`rect_distance`](Self::rect_distance): Chebyshev cells for
    /// `Isometric`, squared Euclidean cells (saturated) for `Orthogonal`.
    pub fn distance_for_rects(self, from: CellRect, to: CellRect) -> u32 {
        let (dx, dy) = rect_gap(from, to);
        match self {
            Projection::Isometric => dx.max(dy),
            Projection::Orthogonal => {
                u32::try_from(dx as u64 * dx as u64 + dy as u64 * dy as u64).unwrap_or(u32::MAX)
            }
        }
    }

    /// The lowest [`rect_distance`](Self::rect_distance) rank any cell can
    /// take on the Chebyshev-shaped scan ring of `radius` — where an
    /// outward ring scan may stop once the floor reaches its best find.
    pub fn ring_floor(self, radius: u32) -> u32 {
        match self {
            Projection::Isometric => radius,
            // Saturated like `rect_distance`, whose ranks it bounds.
            Projection::Orthogonal => {
                u32::try_from(radius as u64 * radius as u64).unwrap_or(u32::MAX)
            }
        }
    }

    /// The length of an offset in this projection's metric, in cells with
    /// sub-cell precision: the dominant axis where diagonals are free, the
    /// Euclidean length where they cost more.
    pub fn span(self, dx: FixedU64, dy: FixedU64) -> FixedU64 {
        match self {
            Projection::Isometric => dx.max(dy),
            Projection::Orthogonal => FixedUVec2::new(dx, dy).length(),
        }
    }

    /// One tick of travel from `position` toward `target`: `speed` cells of
    /// ground under this projection's own metric, never overshooting — the
    /// same geometry the step costs charge for, so a diagonal walk takes
    /// the time its path cost promised.
    ///
    /// The step is the direction scaled to metric length `speed`: under
    /// `Isometric` the dominant axis advances at full rate and the minor
    /// axis proportionally (a pure diagonal costs nothing extra); under
    /// `Orthogonal` the vector is Euclidean-normalized. Single-cell
    /// crossings — cardinal or diagonal — are identical either way; the
    /// proportional minor axis matters for free vectors, where it restores
    /// a deflected mover gently instead of snapping it back in one tick.
    pub fn step_toward(
        self,
        position: FixedUVec2,
        target: FixedUVec2,
        speed: FixedU64,
    ) -> FixedUVec2 {
        let dx = position.x.abs_diff(target.x);
        let dy = position.y.abs_diff(target.y);
        let distance = self.span(dx, dy);
        if distance <= speed {
            return target;
        }
        FixedUVec2::new(
            step_axis(position.x, target.x, dx * speed / distance),
            step_axis(position.y, target.y, dy * speed / distance),
        )
    }

    /// Whether an axis-aligned gap of `dx`/`dy` cells lies within `distance`
    /// under this projection's metric — the one range test every `in_range`
    /// flavor reduces to once its operands' gap is known.
    fn gap_in_range(self, dx: u32, dy: u32, distance: u32) -> bool {
        match self {
            Projection::Isometric => dx.max(dy) <= distance,
            Projection::Orthogonal => {
                let (dx, dy) = (dx as u64, dy as u64);
                let distance = distance as u64;
                dx * dx + dy * dy <= distance * distance
            }
        }
    }
}

/// The per-axis gap in cells between two rectangles, measured cell to cell:
/// zero on an axis where they overlap, one where they abut — the same
/// convention as every other cell distance.
fn rect_gap(a: CellRect, b: CellRect) -> (u32, u32) {
    let axis = |a_min: u32, a_len: u32, b_min: u32, b_len: u32| {
        // The last cell of a rect is `min + len - 1`; rects sharing a cell on
        // this axis give zero.
        let a_max = a_min + a_len - 1;
        let b_max = b_min + b_len - 1;
        // Whichever way round they lie, the gap is the positive difference
        // between one's near edge and the other's far edge; overlapping rects
        // give zero on both.
        a_min.saturating_sub(b_max).max(b_min.saturating_sub(a_max))
    };
    (
        axis(a.origin.x, a.size.width, b.origin.x, b.size.width),
        axis(a.origin.y, a.size.height, b.origin.y, b.size.height),
    )
}

/// Moves `current` toward `target` by at most `step`, without overshooting.
fn step_axis(current: FixedU64, target: FixedU64, step: FixedU64) -> FixedU64 {
    if current < target {
        let next = current + step;
        if next > target { target } else { next }
    } else if current > target {
        if current - target < step {
            target
        } else {
            current - step
        }
    } else {
        current
    }
}

/// Whether `cell` lies within `distance` of the nearest cell of `rect` by the
/// Euclidean metric: the reach of a circle, whatever the map's projection.
pub fn in_circle(cell: CellPos, rect: CellRect, distance: u32) -> bool {
    Projection::Orthogonal.in_range_of_rect(cell, rect, distance)
}

/// Every cell within `distance` of `rect` by the Euclidean metric (see
/// [`in_circle`]), row-major; clamped to non-negative coordinates and
/// unbounded above.
pub fn circle_cells(rect: CellRect, distance: u32) -> Vec<CellPos> {
    Projection::Orthogonal.cells_in_range_of_rect(rect, distance)
}
