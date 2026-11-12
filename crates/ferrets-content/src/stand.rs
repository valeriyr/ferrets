//! Content-defined acts an entity performs once, the tick it comes to stand.

use crate::{
    area::Area,
    field::{FieldAction, FieldId},
};

/// One act an instance performs once, the first tick it stands finished:
/// risen, landed, or placed by the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingAct {
    /// Covers or clears a field around the footprint.
    Field {
        /// The field acted on.
        field: FieldId,
        /// The area around the footprint the act reaches, read when it acts.
        area: Area,
        /// Whether the cells are covered or cleared.
        action: FieldAction,
    },
}
