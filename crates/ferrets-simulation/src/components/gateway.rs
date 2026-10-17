//! Gateway runtime state: how a gateway's entrance is paced.

use bevy_ecs::prelude::*;

/// A gateway's live state: how its entrance is paced.
#[derive(Component, Debug, Default)]
pub struct GatewayComponent {
    /// The earliest tick the next body may enter at. Bodies arriving sooner
    /// wait in place.
    pub boarding_ready_at: u32,
}
