//! What a route is being planned for: the shape that has to fit, and how
//! strictly a query reads the grid.

use crate::mover_shape::MoverShape;

/// Which blockers a navigation query honors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blockers {
    /// Static occupancy and unit claims alike.
    All,
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
}
