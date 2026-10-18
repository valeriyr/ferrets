//! Tunnel end runtime state: the other end it is bound to, and the bore it
//! opens.

use bevy_ecs::prelude::*;

use crate::{bore_id::BoreId, simulation_id::SimulationId};

/// Whether a tunnel end has its other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherEnd {
    /// It has none, and may raise one.
    Unbound,
    /// It is bound to this one, standing or still rising.
    Bound(SimulationId),
}

/// A tunnel end's live state.
#[derive(Component, Debug)]
pub struct TunnelEndComponent {
    /// Its other end.
    pub other: OtherEnd,
    /// The bore it opens, naming a held tunnel's link; kept when its other
    /// end is gone.
    pub bore: BoreId,
}

impl TunnelEndComponent {
    /// An end with no other end, opening `bore` alone.
    pub fn unbound(bore: BoreId) -> Self {
        Self {
            other: OtherEnd::Unbound,
            bore,
        }
    }
}
