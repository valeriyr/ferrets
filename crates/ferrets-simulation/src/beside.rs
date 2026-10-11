//! Where a body coming back onto the map is set down.

use ferrets_geometry::cell_rect::CellRect;

use crate::simulation_id::SimulationId;

/// What a body coming back onto the map is set down beside.
#[derive(Debug, Clone, Copy)]
pub enum Beside {
    /// What it was inside — its holder or its job — as that stands now, or
    /// what that is itself inside; once nothing of it remains in the world,
    /// the footprint a deferred return last searched beside, else where the
    /// body went in.
    Container(SimulationId),
    /// The cell it went in by: it was inside nothing that remains.
    WentIn,
    /// This footprint.
    Footprint(CellRect),
}

/// Which of the free cells in the nearest ring a returning body takes first.
#[derive(Debug, Clone, Copy)]
pub enum Lean {
    /// The one nearest where it went in.
    WentIn,
    /// The one nearest this footprint.
    Toward(CellRect),
    /// None in particular: the ring's own order.
    Nothing,
}
