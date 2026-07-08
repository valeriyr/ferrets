//! A datagram transport over UDP.
//!
//! Connectionless: peers are a static list of `(PeerId, SocketAddr)`, so it
//! serves both a full mesh (every peer's address) and a star (just the host's,
//! or the host knowing every client's). I/O runs on a `tokio` runtime on a
//! background thread; the sim thread drives it through `SocketIo`. UDP is
//! lossy by design — the lockstep driver's redundancy window recovers dropped
//! datagrams, so this layer adds no acknowledgement of its own.

use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::socket_io::{ObservedAddrs, SharedState, SocketIo};
use super::{ConnectionState, NetworkTransport, TransportEvent};
use crate::peer::PeerId;

/// Generous upper bound for one datagram; a frame batch is far smaller.
const MAX_DATAGRAM: usize = 64 * 1024;

/// Binds a gameplay socket. An explicit `port` is used exactly as given — a
/// player who configured a port (a firewall rule, a forwarded port) must play
/// on that port or learn why not, so an occupied port is an error, never a
/// silent substitute. `None` binds an ephemeral port, letting any number of
/// instances share a machine; peers learn the real port through the lobby
/// either way.
pub(crate) fn bind_gameplay_socket(port: Option<u16>) -> std::io::Result<std::net::UdpSocket> {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port.unwrap_or(0)))
}

/// One endpoint of a UDP peer group.
pub struct UdpTransport(SocketIo);

impl UdpTransport {
    /// Binds `addr` and connects to the given peers (each `(PeerId, address)`).
    /// The peer list excludes this endpoint.
    pub fn bind(
        local: PeerId,
        addr: SocketAddr,
        peers: Vec<(PeerId, SocketAddr)>,
    ) -> crate::transport::Result<Self> {
        Self::from_socket(local, std::net::UdpSocket::bind(addr)?, peers)
    }

    /// Like [`bind`](Self::bind) but over an already-bound socket — lets a caller
    /// learn the assigned address (ephemeral port) before wiring up peer lists.
    pub fn from_socket(
        local: PeerId,
        socket: std::net::UdpSocket,
        peers: Vec<(PeerId, SocketAddr)>,
    ) -> crate::transport::Result<Self> {
        socket.set_nonblocking(true)?;

        let peer_ids = peers.iter().map(|(id, _)| *id).collect();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();
        let state = SharedState::default();
        let task_state = state.clone();

        let thread = std::thread::Builder::new()
            .name("ferrets-udp".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build udp runtime");
                runtime.block_on(async move {
                    let socket = UdpSocket::from_std(socket).expect("tokio udp from_std");
                    run(socket, peers, outbound_rx, inbound_tx, task_state).await;
                });
            })?;

        Ok(Self(SocketIo::new(
            local,
            peer_ids,
            outbound_tx,
            inbound_rx,
            state,
            ObservedAddrs::default(),
            thread,
        )))
    }
}

/// The datagram I/O loop: fan out broadcasts to every peer and route inbound
/// datagrams back by source address. Exits when the outbound channel closes.
async fn run(
    socket: UdpSocket,
    peers: Vec<(PeerId, SocketAddr)>,
    mut outbound: UnboundedReceiver<Vec<u8>>,
    inbound: crossbeam_channel::Sender<TransportEvent>,
    state: SharedState,
) {
    for (id, _) in &peers {
        let _ = inbound.send(TransportEvent::PeerConnected(*id));
    }
    state.set(ConnectionState::Connected);

    let by_addr: HashMap<SocketAddr, PeerId> =
        peers.iter().map(|(id, addr)| (*addr, *id)).collect();
    let mut buf = vec![0u8; MAX_DATAGRAM];

    loop {
        tokio::select! {
            outgoing = outbound.recv() => {
                let Some(bytes) = outgoing else { break };
                for (_, addr) in &peers {
                    let _ = socket.send_to(&bytes, addr).await;
                }
            }
            incoming = socket.recv_from(&mut buf) => {
                if let Ok((len, addr)) = incoming
                    && let Some(&from) = by_addr.get(&addr)
                {
                    let event = TransportEvent::Message {
                        from,
                        bytes: buf[..len].to_vec(),
                    };
                    let _ = inbound.send(event);
                }
            }
        }
    }

    state.set(ConnectionState::Disconnected);
}

impl NetworkTransport for UdpTransport {
    fn local_peer(&self) -> PeerId {
        self.0.local_peer()
    }

    fn broadcast(&mut self, bytes: &[u8]) -> crate::transport::Result<()> {
        self.0.broadcast(bytes)
    }

    fn poll(&mut self) -> Vec<TransportEvent> {
        self.0.poll()
    }

    fn peers(&self) -> &[PeerId] {
        self.0.peers()
    }

    fn state(&self) -> ConnectionState {
        self.0.state()
    }
}
