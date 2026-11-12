//! A number content declares as a fixed value or as a stat read from the
//! entity it applies to.

use ferrets_math::FixedU64;

use crate::entity_stats::EntityStatId;

/// A number content declares, counted in whatever unit the field carrying it
/// is measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    /// A fixed value, the same for every entity that reads it.
    Constant(FixedU64),
    /// Read from the entity's effective stats each time it is needed, so the
    /// modifier pipeline can move it while the work is under way.
    Stat(EntityStatId),
}

impl From<u32> for Quantity {
    /// A constant of `value` whole units.
    fn from(value: u32) -> Self {
        Quantity::Constant(FixedU64::from_num(value))
    }
}
