//! A moving thing's physical presence: a circle over the cell grid.

use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, location::Location,
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_nav_grid::layer_mask::LayerMask;

/// One body during a contact-resolution pass: a circle around its position —
/// where it is rendered — with a radius in cells.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    /// The body's place: its middle, in cells with sub-cell precision, is
    /// the circle's center, and its size the footprint it moves as.
    pub location: Location,
    /// The circle's radius, in cells.
    pub radius: FixedU64,
    /// How strongly the body resists displacement, relative to what it meets.
    /// Only the ratio between two bodies' weights matters, never the scale.
    pub weight: FixedU64,
    /// The layers the body occupies; bodies on disjoint layers pass through
    /// each other — including one on no layer, which touches nothing at all.
    pub layers: LayerMask,
    /// Where the body is walking this tick — the offset toward its current
    /// waypoint — or `None` at rest. Contacts consult it for the swerving
    /// traffic rule.
    pub heading: Option<FixedVec2>,
}

/// The whole cells a body at `location` stands on for a reach measure: the
/// hull of its footprint ([`Location::footprint`]) and the footprint its
/// bounds' low corner floors into.
///
/// Both, because a body lying across a cell boundary is standing on the cells
/// either side of it and neither quantization alone says so. A body a little
/// past a footprint's corner floors one cell short on one axis while rounding
/// one cell long on the other, so each single cell reads it as out of reach of
/// what it is plainly standing beside — and the hull, whose gap on an axis is
/// the nearer of the two, does not. A body lined up on its cells floors and
/// rounds to the same cell, so this is exactly its footprint; one part way
/// across a boundary — a continuous mover anywhere, a cell mover mid-crossing —
/// stands on both.
pub fn standing_rect(location: Location) -> CellRect {
    let floored = CellPos::from(location.bounds().min());
    location
        .footprint()
        .union(CellRect::new(floored, location.size()))
}

/// The whole cells the bounds of a body at `location` lie over
/// ([`CellRect::covering`]).
pub fn covered_rect(location: Location) -> CellRect {
    CellRect::covering(location.bounds())
}

/// The whole cells the bounding box of a circle of `radius` around
/// `position` spans: every cell the circle overlaps, and a corner cell it
/// misses. Touching a cell boundary exactly does not enter the next cell;
/// coordinates are unclamped, as [`overlapped_cells`]' are.
pub fn circle_rect(position: FixedUVec2, radius: FixedU64) -> CellRect {
    let span = |center: FixedU64| {
        let low = center.saturating_sub(radius);
        let high = center + radius;
        let first = low.floor().to_num::<u32>();
        let last = if high.frac() == FixedU64::ZERO {
            high.to_num::<u32>().saturating_sub(1)
        } else {
            high.floor().to_num::<u32>()
        };
        // A circle of no radius spans the one cell its middle lies in.
        let last = last.max(first);
        (first, last - first + 1)
    };
    let (x, width) = span(position.x);
    let (y, height) = span(position.y);
    CellRect::new(CellPos::new(x, y), CellSize::new(width, height))
}

/// The cells a circle of `radius` around `position` physically overlaps, in
/// row-major order: a bounding-box corner cell the circle misses does not
/// count, and touching a cell boundary exactly does not enter the next
/// cell. Coordinates are unclamped — a body hanging over the grid's far
/// edge names cells past it, which readers treat as they see fit (the
/// terrain checks read them as impassable).
pub fn overlapped_cells(position: FixedUVec2, radius: FixedU64) -> Vec<CellPos> {
    // Exact on raw bits: overlap requires the circle to reach past the
    // cell's nearest point strictly.
    let nearest = |value: FixedU64, cell: u32| {
        value.clamp(FixedU64::from_num(cell), FixedU64::from_num(cell + 1))
    };
    let reach = radius.to_bits() as u128;
    circle_rect(position, radius)
        .cells()
        .filter(|cell| {
            let off_x = position.x.abs_diff(nearest(position.x, cell.x)).to_bits() as u128;
            let off_y = position.y.abs_diff(nearest(position.y, cell.y)).to_bits() as u128;
            off_x * off_x + off_y * off_y < reach * reach
        })
        .collect()
}
