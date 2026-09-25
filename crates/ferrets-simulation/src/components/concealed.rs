//! Marker for entities a side that is not their own sees only under its
//! detection.

use bevy_ecs::prelude::*;

/// Marks an entity that is not seen by a side that is not its own unless that
/// side's detection covers a cell it stands on.
///
/// Present exactly while the entity's type, an active buff or a field it
/// declares an effect for conceals it.
#[derive(Component, Debug, Default)]
pub struct ConcealedComponent;
