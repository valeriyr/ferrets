//! What a body may not enter: the map's edge, the cells the terrain refuses
//! it, and the circles it is given — one kind of obstacle, judged by one
//! test.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, location::Location, shape::Shape};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_nav_grid::{mover_shape::MoverShape, nav_grid::NavGrid};

use crate::motion;

/// Whether `body`, moving as `mover`, lies wholly on the map and overlaps
/// only cells statically clear for it and joined, for its cliff crossing, to
/// the cell its middle floors into — the start of the cliff-crossing test,
/// not the body's level; for an even-sized body the +x+y one of its middle
/// cells.
pub fn body_fits(grid: &NavGrid, mover: MoverShape, body: Shape) -> bool {
    let position = body.middle();
    let under = CellPos::from(position);
    on_map(grid, mover, position)
        && body
            .cells()
            .into_iter()
            .all(|cell| stands_on(grid, mover, under, cell))
}

/// The ground one body moves over: the grid's terrain for the shape it moves
/// as, and the circles it may not enter.
pub struct Ground<'a> {
    /// The grid whose terrain and edges bound the body.
    grid: &'a NavGrid,
    /// The shape the body moves as on the grid.
    mover: MoverShape,
    /// The body's circle where it stands.
    walker: Shape,
    /// The circles it may not enter.
    obstacles: &'a [Shape],
    /// The cells the body already overlaps that the terrain refuses it: kept,
    /// never spread to new ones.
    clipped: Vec<CellPos>,
    /// The square the body is anchored as where it stands.
    held: CellRect,
}

impl<'a> Ground<'a> {
    /// The ground of a body whose circle is `walker`, moving as `mover` over
    /// `grid`, past the `obstacles` circles.
    pub fn new(
        grid: &'a NavGrid,
        mover: MoverShape,
        walker: Shape,
        obstacles: &'a [Shape],
    ) -> Self {
        let from = CellPos::from(walker.middle());
        let clipped = walker
            .cells()
            .into_iter()
            .filter(|&cell| !stands_on(grid, mover, from, cell))
            .collect();
        let held = CellRect::new(anchor_at(grid, mover, walker.middle()), mover.size);
        Self {
            grid,
            mover,
            walker,
            obstacles,
            clipped,
            held,
        }
    }

    /// The body's circle where it stands.
    pub(crate) fn walker(&self) -> Shape {
        self.walker
    }

    /// The circles the body may not enter.
    pub(crate) fn obstacles(&self) -> &[Shape] {
        self.obstacles
    }

    /// Whether the body may stand with its middle at `to`: wholly on the map;
    /// its circle there overlapping only cells the terrain allows it, or ones
    /// it already overlaps; the square it is anchored as there fitting the
    /// static plane; the cell under it joined to the one it leaves; and sunk
    /// no deeper into any obstacle than it is where it stands.
    pub fn admits(&self, to: FixedUVec2) -> bool {
        let from = CellPos::from(self.walker.middle());
        let under = CellPos::from(to);
        on_map(self.grid, self.mover, to)
            && self.grid.joins(from, under, self.mover)
            && self.walker.moved_to(to).cells().into_iter().all(|cell| {
                stands_on(self.grid, self.mover, under, cell) || self.clipped.contains(&cell)
            })
            && self.grid.fits_statically_from(
                anchor_at(self.grid, self.mover, to),
                self.mover,
                self.held,
            )
            && !self
                .obstacles
                .iter()
                .any(|&obstacle| sinks(self.walker, to, obstacle))
    }
}

/// Whether a walker whose circle is `walker` moved to `to` would overlap the
/// `obstacle` circle deeper than it does where it stands.
///
/// Panics if `walker` or `obstacle` is not a circle.
fn sinks(walker: Shape, to: FixedUVec2, obstacle: Shape) -> bool {
    let touching = motion::reach_of(walker, obstacle);
    let center = obstacle.middle();
    let after = to.distance(center);
    after < touching && after < walker.middle().distance(center)
}

/// The cell a body moving as `mover` with its middle at `position` is
/// anchored at.
fn anchor_at(grid: &NavGrid, mover: MoverShape, position: FixedUVec2) -> CellPos {
    Location::new(position, mover.size, grid.size()).anchor()
}

/// Whether a body moving as `mover` with its middle at `position` lies wholly
/// on the map `grid` covers.
fn on_map(grid: &NavGrid, mover: MoverShape, position: FixedUVec2) -> bool {
    Location::checked(position, mover.size, grid.size()).is_some()
}

/// Whether a body moving as `mover` over `under` may overlap `cell`: the cell
/// is statically clear for it and joined to `under` for its cliff crossing.
fn stands_on(grid: &NavGrid, mover: MoverShape, under: CellPos, cell: CellPos) -> bool {
    grid.cell_fits_statically(cell, mover) && grid.joins(under, cell, mover)
}
