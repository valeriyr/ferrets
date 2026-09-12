//! Selectable movement execution models.

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use serde::{Deserialize, Serialize};

/// How a game's units occupy space and resolve blocking between each other.
///
/// The model is fixed game configuration, identical on every peer for the
/// whole session. It selects the movement execution the simulation runs;
/// long-range pathfinding is shared by every model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MovementModel {
    /// Units claim whole cells, one claimant per cell per layer, and move
    /// cell-to-cell in discrete crossings. Positions stay on the cell
    /// lattice.
    #[default]
    Cell,
    /// Units occupy radius circles and resolve contact by pushing each
    /// other apart; positions are unconstrained points. The claim plane is
    /// derived, rebuilt every tick from the cell under each body's center,
    /// so long-range planning still sees where crowds stand.
    Continuous,
}

/// Returns `true` if `position` is between two cell origins (a crossing is in progress).
///
/// This infers movement state from the position alone, which is only valid
/// under the lattice invariant documented on
/// [`LocationComponent::position`](crate::components::location::LocationComponent):
/// entities rest exactly on cell origins (integer coordinates), so a
/// fractional component can mean nothing but a crossing in progress.
pub fn is_mid_crossing(position: FixedUVec2) -> bool {
    let cell = CellPos::from(position);
    position != FixedUVec2::from(cell)
}
