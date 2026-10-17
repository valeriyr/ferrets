//! The gateway vocabulary: an entity bodies enter to be held inside what it
//! joins or to come out at another of its gateways.

use crate::{network::NetworkId, tunnel::TunnelId};

/// What a gateway joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joins {
    /// A gateway of a registered network.
    Network(NetworkId),
    /// One of the two ends of a registered tunnel, bound at birth to the
    /// unbound end of the same tunnel that built, summoned or trained it.
    Tunnel(TunnelId),
}

impl Joins {
    /// The network joined, or `None` for a tunnel.
    pub fn network(self) -> Option<NetworkId> {
        match self {
            Joins::Network(network) => Some(network),
            Joins::Tunnel(_) => None,
        }
    }

    /// The tunnel joined, or `None` for a network.
    pub fn tunnel(self) -> Option<TunnelId> {
        match self {
            Joins::Tunnel(tunnel) => Some(tunnel),
            Joins::Network(_) => None,
        }
    }
}

/// Which ways a gateway's doors open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doors {
    /// Bodies enter and come out.
    Both,
    /// Bodies enter only.
    Entrance,
    /// Bodies come out only.
    Exit,
}

impl Doors {
    /// Whether bodies may enter by these doors.
    pub fn admits_entrance(self) -> bool {
        match self {
            Doors::Both | Doors::Entrance => true,
            Doors::Exit => false,
        }
    }

    /// Whether bodies may come out by these doors.
    pub fn admits_exit(self) -> bool {
        match self {
            Doors::Both | Doors::Exit => true,
            Doors::Entrance => false,
        }
    }
}

/// Content-defined gateway: a way in or out of what it joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayDef {
    /// What it joins.
    joins: Joins,
    /// Which ways its doors open.
    doors: Doors,
}

impl GatewayDef {
    /// Creates a new `GatewayDef` with the given data.
    pub fn new(joins: Joins, doors: Doors) -> Self {
        Self { joins, doors }
    }

    /// What it joins.
    #[inline]
    pub fn joins(&self) -> Joins {
        self.joins
    }

    /// Which ways its doors open.
    #[inline]
    pub fn doors(&self) -> Doors {
        self.doors
    }
}
