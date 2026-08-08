//! The networked session: a control channel and a gameplay channel.
//!
//! Both channels are always present and independent. How they map to sockets is
//! hidden: a host-star game [`split`](crate::demux::split)s its one socket into
//! the two channels, while a mesh game keeps the reliable control
//! transport and binds a separate UDP transport for gameplay. Either way, the
//! gameplay [`LockstepDriver`] only ever handles frames — control is the control
//! channel's concern.

use std::net::SocketAddr;

use ferrets_simulation::{
    input::PlayerFrame,
    session::{
        ai_hosting::AiHosting,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
};

use crate::{
    control::{ControlChannel, ControlEvent},
    demux,
    driver::{LockstepDriver, Received},
    error::NetworkError,
    lobby::{client::LobbyClient, host::LobbyHost},
    message::control::{ControlMessage, Occupant, SlotInfo, UdpEntry},
    peer::{HOST_PEER, PeerId},
    role::Role,
    roster::Roster,
    session_mode::SessionMode,
    transport::{NetworkTransport, error::TransportError, tcp::TcpTransport, udp::UdpTransport},
};

/// A live networked session.
pub struct NetSession {
    gameplay: LockstepDriver,
    control: ControlChannel,
}

impl NetSession {
    /// Builds a session from a gameplay driver and a control channel that are
    /// already on their own sockets (a mesh game).
    pub fn new(gameplay: LockstepDriver, control: ControlChannel) -> Self {
        Self { gameplay, control }
    }

    /// Builds a session whose control and gameplay channels share one socket (a
    /// host-star game), splitting it into the two channels. `role` is the gameplay
    /// driver's relay role.
    pub fn over_shared(transport: Box<dyn NetworkTransport>, role: Role, roster: Roster) -> Self {
        let (control, gameplay) = demux::split(transport);
        Self::new(
            LockstepDriver::new(gameplay, role, roster),
            ControlChannel::new(control),
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

    /// The player controlled by the session host's node, if that slot exists.
    pub fn host_player(&self) -> Option<PlayerId> {
        self.gameplay.host_player()
    }

    /// The player controlled by transport `peer`, if that slot exists.
    pub fn player_of(&self, peer: PeerId) -> Option<PlayerId> {
        self.gameplay.player_of(peer)
    }

    /// Whether `peer` is the session host's node.
    pub fn is_host_peer(&self, peer: PeerId) -> bool {
        self.gameplay.is_host_peer(peer)
    }

    /// Whether this node holds a direct control link to `player`. False when
    /// `player` is reachable only through a relay — a partial mesh where the
    /// link was never present, or one that has since gone down.
    pub fn has_control_link(&self, player: PlayerId) -> bool {
        self.gameplay
            .peer_of(player)
            .is_some_and(|peer| self.control.peers().contains(&peer))
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

    /// Whether this node is the session host's node ([`HOST_PEER`] — the one
    /// that opened the lobby). What that means in-game depends on the session's
    /// authority; under peer authority it means nothing once the game starts.
    pub fn is_host_node(&self) -> bool {
        self.gameplay.is_host_node()
    }

    /// Sends a control message. A client reaches the host; the host reaches every
    /// client.
    pub fn send_control(&mut self, message: &ControlMessage) -> crate::Result<()> {
        self.control.send(message)
    }

    /// Takes everything the control links carried since the last call: the
    /// messages, plus the players whose control link went down. A control
    /// link is TCP, so its death is a definite event — unlike gameplay
    /// silence — and the session must react to it (an unreachable player can
    /// neither receive decisions nor take part in consensus).
    pub fn drain_control(&mut self) -> ReceivedControl {
        let mut received = ReceivedControl::default();
        for event in self.control.poll() {
            match event {
                ControlEvent::Message { from, message } => received.messages.push((from, message)),
                ControlEvent::Disconnected(peer) => {
                    if let Some(player) = self.gameplay.player_of(peer) {
                        received.lost.push(player);
                    }
                }
                // The roster is fixed once the game starts; a late connect joins nothing.
                ControlEvent::Connected(_) => {}
            }
        }
        received
    }

    /// Builds the host's session and tells the clients to start.
    ///
    /// For a mesh game the host's gameplay socket binds here. An explicit
    /// `udp_port` is used exactly as given (an occupied port is an error — a
    /// configured port must not be silently substituted); `None` binds an
    /// ephemeral port. The real address rides in the distributed endpoint
    /// table either way.
    ///
    /// `slots` is the session's final slot list — the lobby's seats plus any
    /// the game itself adds. Every node starts from the same list, so the
    /// rosters route every slot's frames identically.
    pub fn start_host(
        mut host: LobbyHost,
        udp_port: Option<u16>,
        slots: &[PlayerSlot],
    ) -> crate::Result<Self> {
        let mode = host.mode();
        let roster = roster_for_session(host.slots(), mode.ai_hosting(), slots)?;
        match mode {
            SessionMode::HostStar { .. } => {
                host.start(None, None)?;
                let transport = host.into_control().into_transport();
                Ok(Self::over_shared(transport, Role::Host, roster))
            }
            SessionMode::MeshHosted { .. } => {
                let socket = crate::transport::udp::bind_gameplay_socket(udp_port)
                    .map_err(TransportError::from)?;
                let local_addr = socket.local_addr().map_err(TransportError::from)?;
                let table =
                    host_endpoint_table(&host, local_addr, |peer| host.client_udp_addr(peer))?;
                host.start(Some(table.clone()), None)?;
                let peers = peers_excluding(&table, HOST_PEER);
                let udp = UdpTransport::from_socket(HOST_PEER, socket, peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, host.into_control()))
            }
            SessionMode::MeshDecentralized => {
                let socket = crate::transport::udp::bind_gameplay_socket(udp_port)
                    .map_err(TransportError::from)?;
                let local_addr = socket.local_addr().map_err(TransportError::from)?;
                let udp_table =
                    host_endpoint_table(&host, local_addr, |peer| host.client_udp_addr(peer))?;
                // The host takes part in the control mesh like anyone else: it
                // binds its own listener and distributes the endpoint table.
                let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
                    .map_err(TransportError::from)?;
                let listener_addr = listener.local_addr().map_err(TransportError::from)?;
                let control_table = host_endpoint_table(&host, listener_addr, |peer| {
                    host.client_control_addr(peer)
                })?;
                host.start(Some(udp_table.clone()), Some(control_table.clone()))?;
                // The lobby star has served its purpose; the mesh replaces it.
                drop(host.into_control());
                let dial = peers_excluding(&control_table, HOST_PEER);
                // The host holds the lowest id, so it dials every client and
                // accepts none.
                let mesh = TcpTransport::mesh(HOST_PEER, listener, dial, Vec::new())?;
                let control = ControlChannel::new(Box::new(mesh));
                let peers = peers_excluding(&udp_table, HOST_PEER);
                let udp = UdpTransport::from_socket(HOST_PEER, socket, peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, control))
            }
        }
    }

    /// Builds a client's session once it has received the host's start signal.
    ///
    /// A mesh game reuses the gameplay socket bound at
    /// [`join`](LobbyClient::join), whose port the host already distributed.
    ///
    /// `slots` is the session's final slot list — the lobby's seats plus any
    /// the game itself adds. Every node starts from the same list, so the
    /// rosters route every slot's frames identically.
    pub fn start_client(client: LobbyClient, slots: &[PlayerSlot]) -> crate::Result<Self> {
        let started = client
            .started()
            .ok_or_else(|| internal("client has not received the start signal"))?
            .clone();
        let mode = client
            .state()
            .ok_or_else(|| internal("client has not received the lobby state"))?
            .mode;
        let roster = roster_for_session(client.slots(), mode.ai_hosting(), slots)?;
        let local = client.control_peer();

        match mode {
            SessionMode::HostStar { .. } => {
                // Gameplay shares the control socket; the offered UDP socket
                // and control listener are unused and drop here.
                let (control, _udp, _listener) = client.into_parts();
                Ok(Self::over_shared(
                    control.into_transport(),
                    Role::Client,
                    roster,
                ))
            }
            SessionMode::MeshHosted { .. } => {
                let table = started
                    .udp_table
                    .ok_or_else(|| internal("mesh start carried no udp table"))?;
                let (control, socket, _listener) = client.into_parts();
                let peers = resolve_udp_peers(&table, local, |peer| control.observed_addr(peer));
                let socket = socket
                    .ok_or_else(|| internal("client joined without offering a udp socket"))?;
                let udp = UdpTransport::from_socket(local, socket, peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, control))
            }
            SessionMode::MeshDecentralized => {
                let udp_table = started
                    .udp_table
                    .ok_or_else(|| internal("mesh start carried no udp table"))?;
                let control_table = started
                    .control_table
                    .ok_or_else(|| internal("decentralized start carried no control table"))?;
                let (star, socket, listener) = client.into_parts();
                let udp_peers =
                    resolve_udp_peers(&udp_table, local, |peer| star.observed_addr(peer));
                let control_peers =
                    resolve_udp_peers(&control_table, local, |peer| star.observed_addr(peer));
                // The lobby star has served its purpose; the mesh replaces it.
                drop(star);
                let listener = listener
                    .ok_or_else(|| internal("client joined without offering a control listener"))?;
                // The lower peer id dials: this node dials the higher ids and
                // accepts one link from each lower one.
                let dial: Vec<_> = control_peers
                    .iter()
                    .copied()
                    .filter(|&(peer, _)| peer > local)
                    .collect();
                let accept: Vec<PeerId> = control_peers
                    .iter()
                    .filter(|&&(peer, _)| peer < local)
                    .map(|&(peer, _)| peer)
                    .collect();
                let mesh = TcpTransport::mesh(local, listener, dial, accept)?;
                let control = ControlChannel::new(Box::new(mesh));
                let socket = socket
                    .ok_or_else(|| internal("client joined without offering a udp socket"))?;
                let udp = UdpTransport::from_socket(local, socket, udp_peers)?;
                let gameplay = LockstepDriver::new(Box::new(udp), Role::Peer, roster);
                Ok(Self::new(gameplay, control))
            }
        }
    }
}

/// Everything the control links carried in one drain.
#[derive(Debug, Default)]
pub struct ReceivedControl {
    /// The control messages with the transport peer that actually sent each,
    /// in arrival order. The sender is authenticated by the link it arrived
    /// on, so a decider can reject a message a peer is not entitled to send
    /// (e.g. a client forging an authoritative drop).
    pub messages: Vec<(PeerId, ControlMessage)>,
    /// Players whose control link went down during this drain.
    pub lost: Vec<PlayerId>,
}

/// The roster for the session's slot list: the peer that feeds each slot's
/// frames. A human slot's peer comes from the lobby's matching entry; an AI
/// slot — lobby-configured, or seated by the game after the lobby's — follows
/// the session's AI hosting; a free slot has no peer.
///
/// Errors if a human slot has no connected peer in the lobby — the session
/// and the lobby disagree about who is seated.
fn roster_for_session(
    lobby: &[SlotInfo],
    ai_hosting: AiHosting,
    slots: &[PlayerSlot],
) -> crate::Result<Roster> {
    let peers = slots
        .iter()
        .map(|slot| match slot.player_type() {
            None => Ok(None),
            Some(PlayerType::Ai) => Ok(match ai_hosting {
                AiHosting::Replicated => None,
                AiHosting::Host => Some(HOST_PEER),
            }),
            Some(PlayerType::Human) => lobby
                .iter()
                .find(|info| info.slot == slot.id())
                .and_then(|info| match info.occupant {
                    Occupant::Human { peer } => Some(peer),
                    // Not a human slot, so no peer answers for it here.
                    Occupant::Open | Occupant::Ai | Occupant::Closed => None,
                })
                .map(Some)
                .ok_or_else(|| {
                    internal(&format!(
                        "human session slot {} has no connected peer in the lobby",
                        slot.id(),
                    ))
                }),
        })
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(Roster::from_slots(peers))
}

/// The endpoint table for a mesh game: the host's own `host_addr` plus every
/// connected client's, each looked up through `client_addr` (its gameplay UDP
/// endpoint, or its control-mesh listener). Fails if a client's endpoint is
/// not yet known.
fn host_endpoint_table(
    host: &LobbyHost,
    host_addr: SocketAddr,
    client_addr: impl Fn(PeerId) -> Option<SocketAddr>,
) -> crate::Result<Vec<UdpEntry>> {
    let mut table = vec![UdpEntry {
        peer: HOST_PEER,
        addr: host_addr,
    }];
    for info in host.slots() {
        if let Occupant::Human { peer } = info.occupant {
            if peer == HOST_PEER {
                continue;
            }
            let addr = client_addr(peer)
                .ok_or_else(|| internal("missing endpoint for a connected client"))?;
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

/// Like [`peers_excluding`], additionally resolving entries advertised with an
/// unspecified IP (a peer that could only report its bind address, e.g.
/// `0.0.0.0`): the datagram transport both dials peers and accepts traffic by
/// these exact addresses, so an unroutable entry silently severs the link in
/// both directions. The proven-reachable substitute is the address this node
/// observed for that peer on the control channel.
fn resolve_udp_peers(
    table: &[UdpEntry],
    local: PeerId,
    observed: impl Fn(PeerId) -> Option<SocketAddr>,
) -> Vec<(PeerId, SocketAddr)> {
    table
        .iter()
        .filter(|entry| entry.peer != local)
        .map(|entry| {
            let mut addr = entry.addr;
            if addr.ip().is_unspecified()
                && let Some(reached) = observed(entry.peer)
            {
                addr.set_ip(reached.ip());
            }
            (entry.peer, addr)
        })
        .collect()
}

fn internal(message: &str) -> NetworkError {
    NetworkError::TransportError(TransportError::InternalError(message.into()))
}
