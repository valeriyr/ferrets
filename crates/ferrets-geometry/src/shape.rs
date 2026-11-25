//! A body's exact shape on the map, the cells it lies over and the gap
//! between two shapes.

use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

use crate::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

/// A body's exact shape on the map, in cells. A shape lies within the
/// coordinate space: its far edge at most 2³² − 1 cells on each axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A box: its bounds.
    Box(FixedURect),
    /// A circle.
    Circle {
        /// The circle's middle.
        middle: FixedUVec2,
        /// The circle's radius, in cells.
        radius: FixedU64,
    },
}

impl Shape {
    /// The box of one whole cell.
    pub fn cell(cell: CellPos) -> Self {
        Shape::Box(CellRect::new(cell, CellSize::ONE).bounds())
    }

    /// The shape's middle.
    pub fn middle(self) -> FixedUVec2 {
        match self {
            Shape::Box(bounds) => FixedUVec2::new(
                bounds.min().x + bounds.width() / 2,
                bounds.min().y + bounds.height() / 2,
            ),
            Shape::Circle { middle, .. } => middle,
        }
    }

    /// The radius of a circle; `None` for a box.
    pub fn radius(self) -> Option<FixedU64> {
        match self {
            Shape::Circle { radius, .. } => Some(radius),
            Shape::Box(_) => None,
        }
    }

    /// The same shape with its middle on `middle`: a box keeps its size, and
    /// a circle its radius.
    ///
    /// Panics for a box that would reach past either end of the coordinate
    /// space there.
    pub fn moved_to(self, middle: FixedUVec2) -> Self {
        match self {
            Shape::Box(bounds) => {
                let low = |middle: FixedU64, size: FixedU64| {
                    middle
                        .checked_sub(size / 2)
                        .expect("a box moved to a middle stays clear of the origin")
                };
                let high = |low: FixedU64, size: FixedU64| {
                    low.checked_add(size)
                        .expect("a box moved to a middle fits the coordinate space")
                };
                let min = FixedUVec2::new(
                    low(middle.x, bounds.width()),
                    low(middle.y, bounds.height()),
                );
                Shape::Box(FixedURect::new(
                    min,
                    FixedUVec2::new(high(min.x, bounds.width()), high(min.y, bounds.height())),
                ))
            }
            Shape::Circle { radius, .. } => Shape::Circle { middle, radius },
        }
    }

    /// Every cell the shape lies over, and for a circle the corner cells of
    /// its box it misses: an edge on a cell boundary enters no cell beyond
    /// it, and a circle of no radius lies over the cell its middle lies in.
    ///
    /// Overflows for a shape whose far edge lies past 2³² − 1 cells: a debug
    /// build panics, a release build wraps.
    pub fn cell_bounds(self) -> CellRect {
        CellRect::covering(self.bounds())
    }

    /// The cells the shape overlaps, in row-major order: those it shares area
    /// with, an edge on a cell boundary entering no cell beyond it; a circle
    /// of no radius overlaps the cell its middle lies in. Coordinates are
    /// unclamped — a shape hanging over the map's far edge names cells past
    /// it.
    ///
    /// Overflows as [`Shape::cell_bounds`] does.
    pub fn cells(self) -> Vec<CellPos> {
        match self {
            Shape::Box(_) => self.cell_bounds().cells().collect(),
            Shape::Circle { radius, .. } if radius == FixedU64::ZERO => {
                self.cell_bounds().cells().collect()
            }
            Shape::Circle { middle, radius } => {
                let reach = bits(radius);
                self.cell_bounds()
                    .cells()
                    .filter(|&cell| {
                        let nearest = Shape::cell(cell).nearest_point(middle);
                        squared_length(middle, nearest) < reach * reach
                    })
                    .collect()
            }
        }
    }

    /// The gap between this shape and `other`, edge to edge, Euclidean,
    /// rounded down, so a gap just past a range can read as that range:
    /// `0` when they touch or overlap. [`Shape::within`] is exact.
    ///
    /// Panics when the shapes' cores lie 2³² cells or more apart, as
    /// [`FixedUVec2::length`] does.
    pub fn distance(self, other: Shape) -> FixedU64 {
        let (squared, radii) = self.span_to(other);
        let gap = squared.isqrt().saturating_sub(radii);
        FixedU64::from_bits(
            u64::try_from(gap).expect("a gap between shapes fits the coordinate space"),
        )
    }

    /// Whether the gap between this shape and `other` is at most `range`,
    /// judged exactly: the range check between two shapes.
    ///
    /// Panics as [`Shape::distance`] does.
    pub fn within(self, other: Shape, range: FixedU64) -> bool {
        let (squared, radii) = self.span_to(other);
        let reach = radii + bits(range);
        squared <= reach.saturating_mul(reach)
    }

    /// The point of the shape nearest `point`: `point` itself when the shape
    /// holds it. On a circle the point is exact to a raw bit per axis, and
    /// may lie that far off the circle's edge.
    ///
    /// Panics as [`Shape::distance`] does, for a circle and a point that far
    /// apart.
    pub fn nearest_point(self, point: FixedUVec2) -> FixedUVec2 {
        match self {
            Shape::Box(bounds) => FixedUVec2::new(
                point.x.clamp(bounds.min().x, bounds.max().x),
                point.y.clamp(bounds.min().y, bounds.max().y),
            ),
            Shape::Circle { middle, radius } => {
                let squared = squared_length(middle, point);
                let reach = bits(radius);
                if squared <= reach * reach {
                    return point;
                }
                let length = squared.isqrt();
                let toward = |from: FixedU64, to: FixedU64| {
                    let offset = bits(from.abs_diff(to)) * reach / length;
                    let offset = FixedU64::from_bits(offset as u64);
                    if to >= from {
                        from + offset
                    } else {
                        from - offset
                    }
                };
                FixedUVec2::new(toward(middle.x, point.x), toward(middle.y, point.y))
            }
        }
    }

    /// The rectangle the shape lies within.
    fn bounds(self) -> FixedURect {
        match self {
            Shape::Box(bounds) => bounds,
            Shape::Circle { middle, radius } => FixedURect::new(
                FixedUVec2::new(
                    middle.x.saturating_sub(radius),
                    middle.y.saturating_sub(radius),
                ),
                FixedUVec2::new(middle.x + radius, middle.y + radius),
            ),
        }
    }

    /// The squared length, in raw bits, between this shape's core and
    /// `other`'s, and the radii the cores are wrapped in: a box is its own
    /// core with no radius, a circle its middle wrapped in its radius.
    fn span_to(self, other: Shape) -> (u128, u128) {
        let core = |shape: Shape| match shape {
            Shape::Box(bounds) => (bounds, 0),
            Shape::Circle { middle, radius } => (FixedURect::new(middle, middle), bits(radius)),
        };
        let ((mine, my_radius), (theirs, their_radius)) = (core(self), core(other));
        let gap = |my_low: FixedU64, my_high: FixedU64, low: FixedU64, high: FixedU64| {
            bits(low.saturating_sub(my_high)) + bits(my_low.saturating_sub(high))
        };
        let x = gap(mine.min().x, mine.max().x, theirs.min().x, theirs.max().x);
        let y = gap(mine.min().y, mine.max().y, theirs.min().y, theirs.max().y);
        (squared_sum(x, y), my_radius + their_radius)
    }
}

/// The raw bits of `value`, widened.
fn bits(value: FixedU64) -> u128 {
    u128::from(value.to_bits())
}

/// The squared length, in raw bits, between two points.
fn squared_length(from: FixedUVec2, to: FixedUVec2) -> u128 {
    squared_sum(bits(from.x.abs_diff(to.x)), bits(from.y.abs_diff(to.y)))
}

/// `x² + y²` of two raw offsets, each below 2⁶⁴.
///
/// Panics when the sum reaches 2¹²⁸: offsets whose length is 2⁶⁴ raw bits
/// (2³² cells) or more.
fn squared_sum(x: u128, y: u128) -> u128 {
    (x * x)
        .checked_add(y * y)
        .expect("offsets this long do not fit the coordinate space")
}
