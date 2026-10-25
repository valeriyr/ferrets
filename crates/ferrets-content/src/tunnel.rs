//! The tunnel vocabulary: a registered passage joining two gateways bound to
//! each other at birth.

use crate::{affiliation::Affiliation, kinds::Kinds, transport::PassengerFate};

/// A handle to a registered tunnel, assigned in registration order.
///
/// Content declares tunnels by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TunnelId(u16);

impl TunnelId {
    /// Creates a tunnel id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more tunnels registered than TunnelId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What becomes of a body that enters one end of a tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passage {
    /// The tunnel holds it until it is let out at either end whose doors admit
    /// an exit.
    Held {
        /// The cargo size the bodies one tunnel holds may add up to.
        capacity: u32,
        /// What happens to the bodies held when both ends are lost.
        fate: PassengerFate,
    },
    /// It steps out at once within the other end's unload range.
    Crosses,
}

/// What happens to one end of a tunnel when its standing other end is lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherEndLost {
    /// It is destroyed with its other end.
    Razed,
    /// It stands unbound, free to raise a new other end.
    Stands,
}

/// A tunnel: a way between two gateway ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelDef {
    /// Whom it lets in.
    carries: Kinds,
    /// Whose units may enter.
    boarding: Affiliation,
    /// What happens to one end when its standing other end is lost.
    other_end_lost: OtherEndLost,
    /// What becomes of a body that enters.
    passage: Passage,
}

impl TunnelDef {
    /// Creates a new `TunnelDef` with the given data.
    pub fn new(
        carries: Kinds,
        boarding: Affiliation,
        other_end_lost: OtherEndLost,
        passage: Passage,
    ) -> Self {
        Self {
            carries,
            boarding,
            other_end_lost,
            passage,
        }
    }

    /// Whom it lets in.
    #[inline]
    pub fn carries(&self) -> &Kinds {
        &self.carries
    }

    /// Whose units may enter.
    #[inline]
    pub fn boarding(&self) -> Affiliation {
        self.boarding
    }

    /// What happens to one end when its standing other end is lost.
    #[inline]
    pub fn other_end_lost(&self) -> OtherEndLost {
        self.other_end_lost
    }

    /// What becomes of a body that enters.
    #[inline]
    pub fn passage(&self) -> Passage {
        self.passage
    }
}
