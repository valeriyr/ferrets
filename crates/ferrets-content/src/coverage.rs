//! How much of a footprint must answer a question for the answer to stand.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect};

/// How much of a footprint must answer a question for the answer to stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// Every cell of the footprint.
    Every,
    /// Any one cell of it.
    Any,
}

impl Coverage {
    /// Whether enough of `rect` answers `test` for this coverage.
    pub fn covers(self, rect: CellRect, test: impl Fn(CellPos) -> bool) -> bool {
        match self {
            Coverage::Every => rect.cells().all(test),
            Coverage::Any => rect.cells().any(test),
        }
    }
}
