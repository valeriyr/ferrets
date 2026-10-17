//! The network vocabulary: a registered passage holding whoever enters any of its
//! gateways, until it is let out at another.

use crate::{affiliation::Affiliation, kinds::Kinds, transport::PassengerFate};

/// A handle to a registered network, assigned in registration order.
///
/// Content declares networks by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkId(u16);

impl NetworkId {
    /// Creates a network id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more networks registered than NetworkId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A network: one hold its gateways share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkDef {
    /// Whom it lets in.
    carries: Kinds,
    /// Whose units may enter a gateway, judged from its owner; the gateways
    /// whose owners admit one another form one running network.
    boarding: Affiliation,
    /// The cargo size the bodies inside one running network may add up to.
    capacity: u32,
    /// What happens to the bodies inside a running network when its last
    /// gateway is lost.
    fate: PassengerFate,
}

impl NetworkDef {
    /// Creates a new `NetworkDef` with the given data.
    pub fn new(carries: Kinds, boarding: Affiliation, capacity: u32, fate: PassengerFate) -> Self {
        Self {
            carries,
            boarding,
            capacity,
            fate,
        }
    }

    /// Whom it lets in.
    #[inline]
    pub fn carries(&self) -> &Kinds {
        &self.carries
    }

    /// Whose units may enter a gateway, judged from its owner; the gateways
    /// whose owners admit one another form one running network.
    #[inline]
    pub fn boarding(&self) -> Affiliation {
        self.boarding
    }

    /// The cargo size the bodies inside one running network may add up to.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// What happens to the bodies inside a running network when its last
    /// gateway is lost.
    #[inline]
    pub fn fate(&self) -> PassengerFate {
        self.fate
    }
}
