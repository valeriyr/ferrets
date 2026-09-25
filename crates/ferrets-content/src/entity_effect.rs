//! What applies to an entity while something holds over it — a buff it
//! carries, a field it stands in.

use crate::stats::EntityModifier;

/// One consequence for an entity while a buff or a field applies to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityEffect {
    /// Folds the modifiers into the entity's effective stats.
    Modifiers(Vec<EntityModifier>),
    /// The entity stands but does not operate: it starts no order but Train
    /// and Research, which wait, and neither fights, hunts, casts nor moves.
    Disable,
    /// The entity is not seen by a side that is not its own unless that
    /// side's detection covers a cell it stands on.
    Conceal,
}
