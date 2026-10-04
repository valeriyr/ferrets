//! Modifiers laid over an entity together, and what they do to its pools.

use crate::{pool_shift::PoolShift, stats::EntityModifier};

/// Modifiers laid over an entity together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityModifiers {
    /// Modifiers on stats no pool takes its maximum from.
    Stats(Vec<EntityModifier>),
    /// Modifiers on pool maxima, moving the pools under them by `pool_shift`.
    PoolMaximums {
        /// The modifiers, each on a pool's maximum.
        modifiers: Vec<EntityModifier>,
        /// What a maximum they move does to the pool under it.
        pool_shift: PoolShift,
    },
}

impl EntityModifiers {
    /// The modifiers, whatever they move.
    pub fn modifiers(&self) -> &[EntityModifier] {
        match self {
            EntityModifiers::Stats(modifiers) | EntityModifiers::PoolMaximums { modifiers, .. } => {
                modifiers
            }
        }
    }
}
