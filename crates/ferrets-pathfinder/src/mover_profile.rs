//! What a route is being planned for: the shape that has to fit, and how
//! strictly a query reads the grid.

use ferrets_geometry::cell_pos::CellPos;
use ferrets_nav_grid::{mover_shape::MoverShape, nav_grid::NavGrid};

/// Which blockers a navigation query honors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blockers {
    /// Static occupancy and every unit claim but the one holder's whose
    /// route is sought.
    AllBut(u32),
    /// Static occupancy only — unit claims are ignored.
    Static,
}

/// The terms a navigation query runs on: the shape to route and the blockers
/// to honor. Not the mover itself — the same mover runs queries under
/// different blocker modes in different phases of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MoverProfile {
    /// The geometry that has to fit.
    pub shape: MoverShape,
    /// How strictly the grid is read.
    pub blockers: Blockers,
}

impl MoverProfile {
    /// Creates a profile from a shape and a blocker mode.
    pub fn new(shape: MoverShape, blockers: Blockers) -> Self {
        Self { shape, blockers }
    }

    /// Returns `true` if the profile's footprint at `anchor` on `grid` is open
    /// under its blocker mode.
    pub(crate) fn fits(self, grid: &NavGrid, anchor: CellPos) -> bool {
        match self.blockers {
            Blockers::AllBut(holder) => grid.fits_for(anchor, self.shape, holder),
            Blockers::Static => grid.fits_statically(anchor, self.shape),
        }
    }

    /// Returns `true` if one cell of the profile's shape at `cell` on `grid`
    /// is open under its blocker mode.
    pub(crate) fn cell_fits(self, grid: &NavGrid, cell: CellPos) -> bool {
        match self.blockers {
            Blockers::AllBut(holder) => grid.cell_fits_for(cell, self.shape, holder),
            Blockers::Static => grid.cell_fits_statically(cell, self.shape),
        }
    }
}
