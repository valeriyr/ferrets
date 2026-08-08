//! Chase progress shared by every order that walks toward something.

use ferrets_geometry::cell_pos::CellPos;

/// `(chaser cell, destination cell)` recorded when the last chase move
/// started. When both are unchanged on resume the chase made no progress and
/// never will. For a stationary destination only the chaser cell can change,
/// so this reduces to tracking the chaser; for a moving destination it gives
/// up only when neither has moved.
///
/// Cells, not positions: a continuous mover's position wobbles by bits under
/// pushing, so exact positions would never repeat and a chase against an
/// unreachable destination would re-walk forever instead of giving up.
pub type ChaseState = Option<(CellPos, CellPos)>;
