//! The lobby/control protocol: messages a host and its clients exchange to agree
//! on the session before it starts (and to steer it afterwards).
//!
//! These ride the reliable control channel. Gameplay frames and checksums are a
//! separate concern ([`Message`](super::Message)).

use std::net::SocketAddr;

use ferrets_simulation::session::player_slot::PlayerId;
use serde::{Deserialize, Serialize};

use crate::{peer::PeerId, topology::Topology};

/// Who occupies a player slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Occupant {
    /// Empty and waiting for a human to join; becomes [`Human`](Self::Human) on
    /// connect, and counts as closed if still empty when the game starts.
    Open,
    /// A networked human, identified by the peer that controls it.
    Human { peer: PeerId },
    /// A locally-computed AI; no peer, no presence on the wire.
    Ai,
    /// A disabled slot: no player, no base, excluded from the running game.
    Closed,
}

/// One player slot's state in the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotInfo {
    /// The slot's dense [`PlayerId`].
    pub slot: PlayerId,
    /// Who fills the slot.
    pub occupant: Occupant,
    /// The chosen race id, if any.
    pub race: Option<String>,
}

/// A peer's gameplay (UDP) endpoint, distributed to every peer before a mesh game
/// starts so each can address the others directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpEntry {
    pub peer: PeerId,
    pub addr: SocketAddr,
}

/// A lobby-coordination message, exchanged while configuring the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyMessage {
    /// Client → host on connect: the client's build identity (the host refuses a
    /// mismatch — see [`ferrets_simulation::PROTOCOL_VERSION`]), the UDP port the
    /// client will receive gameplay on for a direct mesh (`None` if it offers none
    /// — used only when the host picks a mesh topology), and an optional preferred
    /// race.
    Join {
        protocol_version: String,
        advertised_udp_port: Option<u16>,
        race: Option<String>,
    },
    /// Host → all, re-sent on every change: the authoritative lobby state. A
    /// client finds its own slot by the peer id it was assigned on connect. Peers
    /// mirror it so their view is always current and their config is built before
    /// the game starts.
    LobbyState {
        slots: Vec<SlotInfo>,
        topology: Topology,
    },
    /// Host → all: the named peer was refused (e.g. a build mismatch). Only the
    /// rejected client acts on it; the others ignore it.
    Rejected { peer: PeerId, reason: String },
    /// Client → host: a request to set a slot's race. The host validates it and
    /// re-broadcasts [`LobbyState`](Self::LobbyState).
    RequestRace { slot: PlayerId, race: String },
    /// Host → all: lock the lobby and begin. The state is already synced, so this
    /// carries only what the lobby broadcasts did not — the UDP endpoint table,
    /// present only for a mesh game.
    Start { udp_table: Option<Vec<UdpEntry>> },
}

/// An in-game control message, exchanged while the session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InGameMessage {
    /// Any node → host: a request to pause (`true`) or resume (`false`). The host
    /// turns it into an authoritative [`PauseAt`](Self::PauseAt).
    PauseRequest { paused: bool },
    /// Host → all: pause (`true`) or resume (`false`) the session, effective at
    /// `tick` on every node so the change is deterministic.
    PauseAt { tick: u32, paused: bool },
}

/// A message on the control channel, before or during the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMessage {
    /// Lobby coordination, before the game starts.
    Lobby(LobbyMessage),
    /// Game control, after the game starts.
    InGame(InGameMessage),
}
