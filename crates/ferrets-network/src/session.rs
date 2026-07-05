//! The networked session: a control channel and a gameplay channel.
//!
//! Both channels are always present and independent. How they map to sockets is
//! hidden: a host-star game [`split`](crate::demux::split)s its one socket into
//! the two channels, while a mesh game keeps the reliable control
//! transport and binds a separate UDP transport for gameplay. Either way, the
//! gameplay [`LockstepDriver`] only ever handles frames — control is the control
//! channel's concern.

use std::net::SocketAddr;

use ferrets_simulation::input::PlayerFrame;
use ferrets_simulation::session::ai_hosting::AiHosting;
use ferrets_simulation::session::player_slot::PlayerId;

use crate::control::{ControlChannel, ControlEvent};
use crate::demux;
use crate::driver::{LockstepDriver, Received};
use crate::error::NetworkError;
use crate::lobby::client::LobbyClient;
use crate::lobby::host::LobbyHost;
use crate::message::control::{ControlMessage, Occupant, SlotInfo, UdpEntry};
use crate::peer::PeerId;
use crate::role::Role;
use crate::roster::Roster;
use crate::topology::Topology;
use crate::transport::NetworkTransport;
use crate::transport::error::TransportError;
use crate::transport::udp::UdpTransport;

/// A live networked session.
pub struct NetSession {
    gameplay: LockstepDriver,
    control: ControlChannel,
    /// Whether this node is the control-plane host (peer `0`). The control plane is
    /// a host-coordinated star in every topology, so the host is the authoritative
    /// emitter of [`PauseAt`](crate::message::control::InGameMessage::PauseAt) and
    /// the like.
    control_host: bool,
}

impl NetSession {
    /// Builds a session from a gameplay driver and a control channel that are
    /// already on their own sockets (a mesh game).
    pub fn new(gameplay: LockstepDriver, control: ControlChannel, control_host: bool) -> Self {
        Self {
            gameplay,
            control,
            control_host,
        }
    }

    /// Builds a session whose control and gameplay channels share one socket (a
    /// host-star game), splitting it into the two channels. `role` is the gameplay
    /// driver's relay role.
    pub fn over_shared(
        transport: Box<dyn NetworkTransport>,
        role: Role,
        roster: Roster,
        control_host: bool,
    ) -> Self {
        let (control, gameplay) = demux::split(transport);
        Self::new(
            LockstepDriver::new(gameplay, role, roster),
            ControlChannel::new(control),
            control_host,
        )
    }

    /// The gameplay channel: frames and checksums.
    pub fn gameplay(&mut self) -> &mut LockstepDriver {
        &mut self.gameplay
    }

    /// The gameplay channel, immutably.
    pub fn gameplay_ref(&self) -> &LockstepDriver {
        &self.gameplay
    }

    /// Whether this node relays other players' frames.
    pub fn relays(&self) -> bool {
        self.gameplay.relays()
    }

    /// Whether `player` is backed by a network peer.
    pub fn is_networked(&self, player: PlayerId) -> bool {
        self.gameplay.is_networked(player)
    }

    /// Broadcasts the given frames on the gameplay channel.
    pub fn broadcast_frames(&mut self, frames: Vec<PlayerFrame>) -> crate::Result<()> {
        self.gameplay.broadcast_frames(frames)
    }

    /// Broadcasts a state checksum on the gameplay channel.
    pub fn send_checksum(&mut self, tick: u32, hash: u64) -> crate::Result<()> {
        self.gameplay.send_checksum(tick, hash)
    }

    /// Drains everything received on the gameplay channel since the last call.
    pub fn drain_received(&mut self) -> Received {
        self.gameplay.drain_received()
    }

    /// Whether this node is the control-plane host (the authoritative emitter of
    /// in-game control like pause).
    pub fn is_control_host(&self) -> bool {
        self.control_host
    }

    /// Sends a control message. A client reaches the host; the host reaches every
    /// client.
    pub fn send_control(&mut self, message: &ControlMessage) -> crate::Result<()> {
        self.control.send(message)
    }

    /// Takes the control messages received since the last call.
    pub fn drain_control(&mut self) -> Vec<ControlMessage> {
        self.control
            .poll()
            .into_iter()
            .filter_map(|event| match event {
                ControlEvent::Message { message, .. } => Some(message),
                _ => None,
            })
            .collect()
    }

    /// Builds the host's session and tells the clients to start.
    ///
    /// `local_udp_bind` is where the host's gameplay socket binds for a mesh game
    /// (ignored for host-star).
    pub fn start_host(mut host: LobbyHost, local_udp_bind: SocketAddr) -> crate::Result<Self> {
        let roster = roster_from_slots(host.slots(), host.ai_hosting());
        match host.topology() {
            Topology::HostStar => {
                host.start(None)?;
                let transport = host.into_control().into_transport();
                Ok(Self::over_shared(transport, Role::Host, roster, true))
            }
            Topology::Mesh => {
                let table = host_udp_table(&host, local_udp_bind)?;
                host.start(Some(table.clone()))?;
                let peers = peers_excluding(&table, HOST_PEER);
                let udp = UdpTransport::bind(HOST_PEER, local_udp_bind, peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, host.into_control(), true))
            }
        }
    }

    /// Builds a client's session once it has received the host's start signal.
    ///
    /// `local_udp_bind` is where this client's gameplay socket binds for a mesh
    /// game (ignored for host-star); it must match the port advertised on join.
    pub fn start_client(client: LobbyClient, local_udp_bind: SocketAddr) -> crate::Result<Self> {
        let udp_table = client
            .started()
            .ok_or_else(|| internal("client has not received the start signal"))?
            .udp_table
            .clone();
        let roster = roster_from_slots(client.slots(), client.ai_hosting());
        let local = client.control_peer();

        match client.topology() {
            Topology::HostStar => {
                let transport = client.into_control().into_transport();
                Ok(Self::over_shared(transport, Role::Client, roster, false))
            }
            Topology::Mesh => {
                let table = udp_table.ok_or_else(|| internal("mesh start carried no udp table"))?;
                let peers = peers_excluding(&table, local);
                let udp = UdpTransport::bind(local, local_udp_bind, peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, client.into_control(), false))
            }
        }
    }
}

/// The host's peer id.
const HOST_PEER: PeerId = 0;

/// Builds the roster from the locked slots: a human slot maps to its peer; an
/// open or closed slot has no network peer. An AI slot depends on the hosting
/// mode — no peer when every node computes it locally, the host peer when the
/// host computes it and broadcasts its frames. The slot index is its [`PlayerId`].
fn roster_from_slots(slots: &[SlotInfo], ai_hosting: AiHosting) -> Roster {
    Roster::from_slots(
        slots
            .iter()
            .map(|info| match info.occupant {
                Occupant::Human { peer } => Some(peer),
                Occupant::Ai => match ai_hosting {
                    AiHosting::Replicated => None,
                    AiHosting::HostOnly => Some(HOST_PEER),
                },
                Occupant::Open | Occupant::Closed => None,
            })
            .collect(),
    )
}

/// The UDP endpoint table for a mesh game: the host's own address plus every
/// connected client's. Fails if a client's endpoint is not yet known.
fn host_udp_table(host: &LobbyHost, host_addr: SocketAddr) -> crate::Result<Vec<UdpEntry>> {
    let mut table = vec![UdpEntry {
        peer: HOST_PEER,
        addr: host_addr,
    }];
    for info in host.slots() {
        if let Occupant::Human { peer } = info.occupant {
            if peer == HOST_PEER {
                continue;
            }
            let addr = host
                .client_udp_addr(peer)
                .ok_or_else(|| internal("missing udp endpoint for a connected client"))?;
            table.push(UdpEntry { peer, addr });
        }
    }
    Ok(table)
}

/// The table entries other than `local`, as the `(peer, addr)` list a transport
/// binds against.
fn peers_excluding(table: &[UdpEntry], local: PeerId) -> Vec<(PeerId, SocketAddr)> {
    table
        .iter()
        .filter(|entry| entry.peer != local)
        .map(|entry| (entry.peer, entry.addr))
        .collect()
}

fn internal(message: &str) -> NetworkError {
    NetworkError::TransportError(TransportError::InternalError(message.into()))
}
