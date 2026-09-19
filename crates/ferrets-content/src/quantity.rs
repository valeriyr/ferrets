//! A number content declares as a fixed value or as a stat read from the
//! entity it applies to.

use crate::entity_stats::EntityStatId;

/// A number content declares, counted in whatever unit the field carrying it
/// is measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    /// A fixed value, the same for every entity that reads it.
    Constant(u32),
    /// Read from the entity's effective stats each time it is needed, so the
    /// modifier pipeline can move it while the work is under way.
    Stat(EntityStatId),
}
