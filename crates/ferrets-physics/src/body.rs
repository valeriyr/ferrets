//! A moving thing's physical presence: a circle over the cell grid.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, location::Location, shape::Shape};
use ferrets_math::{FixedU64, fixed_vec2::FixedVec2};
use ferrets_nav_grid::mover_shape::MoverShape;

/// What a body is doing, as contact reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bearing {
    /// Walking, the way its walk heads.
    Walking(Heading),
    /// At work in place.
    Working,
    /// Holding its place.
    Planted,
    /// Unable to move at all.
    Fixed,
    /// Displaced as an idle body is, keeping no spacing.
    Inert,
    /// At rest.
    Idle,
}

/// The way a walking body's walk heads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    /// The offset toward where the walk aims.
    Toward(FixedVec2),
    /// The walk aims nowhere: no plan yet, or its aim is where it stands.
    Unaimed,
}

impl Heading {
    /// The heading toward a point at `offset` from the walker: nowhere for a
    /// zero offset.
    pub fn toward(offset: FixedVec2) -> Self {
        if offset == FixedVec2::ZERO {
            Heading::Unaimed
        } else {
            Heading::Toward(offset)
        }
    }
}

/// How far contact may displace a body, lowest rank first: a body of a lower
/// rank gives way to one of a higher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    /// Displaced as its meeting and weight say.
    Free,
    /// Displaced by nothing but another working, a planted or a fixed body.
    Working,
    /// Displaced by nothing but another planted or a fixed body.
    Planted,
    /// Displaced by nothing.
    Fixed,
}

impl Bearing {
    /// The rank of a body of this bearing.
    pub fn rank(self) -> Rank {
        match self {
            Bearing::Fixed => Rank::Fixed,
            Bearing::Working => Rank::Working,
            Bearing::Planted => Rank::Planted,
            Bearing::Walking(_) | Bearing::Inert | Bearing::Idle => Rank::Free,
        }
    }
}

/// One body in contact: a circle around its position — where it is rendered —
/// with a radius in cells.
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
    /// The shape it moves as on the grid: the terrain it may cover, the
    /// layers it occupies — bodies on disjoint layers pass through each
    /// other — and how it crosses cliffs.
    pub mover: MoverShape,
    /// What the body is doing, as contact reads it.
    pub bearing: Bearing,
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
