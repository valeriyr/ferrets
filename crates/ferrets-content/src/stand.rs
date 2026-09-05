//! Content-defined acts an entity performs once, the tick it comes to stand.

use crate::field::{FieldAction, FieldId};

/// One act an instance performs once, the first tick it stands finished:
/// complete, landed, or placed by the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingAct {
    /// Covers or clears a field around the footprint.
    Field {
        /// The field acted on.
        field: FieldId,
        /// How far from the footprint the act reaches, in cells.
        radius: u32,
        /// Whether the cells are covered or cleared.
        action: FieldAction,
    },
}
