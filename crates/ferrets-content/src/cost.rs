//! What an entity pays, from whichever pool the price is drawn.

use ferrets_math::FixedU64;

use crate::price::Price;

/// One price an entity pays, drawn from the pool its arm names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cost {
    /// Resource kinds from the owner's stockpile.
    Resources(Price),
    /// The entity's energy pool.
    Energy(FixedU64),
    /// The entity's own health — a price that could not be survived is
    /// refused.
    Health(FixedU64),
}
