//! How a session routes gameplay frames between peers.

use serde::{Deserialize, Serialize};

/// How the session routes gameplay frames once it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topology {
    /// Every frame relays through the host (a star).
    HostStar,
    /// Peers exchange frames directly (a mesh).
    Mesh,
}
