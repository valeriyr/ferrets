//! The session mode a lobby agrees on.

use ferrets_simulation::session::{ai_hosting::AiHosting, authority::Authority};
use serde::{Deserialize, Serialize};

use crate::topology::Topology;

/// How a session is wired and governed, as one choice. Each variant is a
/// complete, coherent mode — the choices that require a host only exist on
/// the variants that have one, so a contradictory configuration cannot be
/// written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    /// Gameplay routes through the host, which also decides drops and pauses.
    /// The session lives exactly as long as the host does.
    HostStar { ai_hosting: AiHosting },
    /// Gameplay flows peer-to-peer, but the host still decides drops and
    /// pauses (and may compute the AI). The session ends if the host leaves.
    ///
    /// Gameplay peers address each other by the endpoints the host distributes,
    /// so every peer must be routable from every other (a LAN or public
    /// addresses); peers behind separate NATs are not reachable directly, and a
    /// host-relayed mode ([`HostStar`](Self::HostStar)) is the path for them
    /// until NAT traversal exists.
    MeshHosted { ai_hosting: AiHosting },
    /// Gameplay flows peer-to-peer and, once the game starts, no node is
    /// special: decisions commit by consensus and the session survives any
    /// single node. AI is necessarily computed on every node.
    ///
    /// Both channels are direct peer-to-peer, so this has the same routability
    /// requirement as [`MeshHosted`](Self::MeshHosted) and adds the control
    /// mesh to it: every peer must be mutually routable, since the addresses
    /// exchanged are the ones the host observed and are not valid across
    /// separate NATs.
    MeshDecentralized,
}

impl SessionMode {
    /// The wire shape gameplay traffic uses in this mode.
    pub fn topology(&self) -> Topology {
        match self {
            Self::HostStar { .. } => Topology::HostStar,
            Self::MeshHosted { .. } | Self::MeshDecentralized => Topology::Mesh,
        }
    }

    /// Who resolves session-level decisions in this mode.
    pub fn authority(&self) -> Authority {
        match self {
            Self::HostStar { ai_hosting } | Self::MeshHosted { ai_hosting } => Authority::Host {
                ai_hosting: *ai_hosting,
            },
            Self::MeshDecentralized => Authority::Peers,
        }
    }

    /// How AI player input is computed in this mode.
    pub fn ai_hosting(&self) -> AiHosting {
        self.authority().ai_hosting()
    }
}
