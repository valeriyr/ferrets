//! A moving thing's physical presence: a circle over the cell grid.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, location::Location, shape::Shape};
use ferrets_math::{FixedU64, fixed_vec2::FixedVec2};
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

impl Body {
    /// The body's circle: its radius around its middle.
    pub fn shape(&self) -> Shape {
        Shape::Circle {
            middle: self.location.middle(),
            radius: self.radius,
        }
    }
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
