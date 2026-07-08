//! A TCP star transport: a host accepts clients, each client dials the host.
//!
//! The host is peer `0`; it assigns each client an ascending [`PeerId`] on
//! connect (sent as an 8-byte little-endian prelude) and thereafter exchanges
//! length-prefixed (`u32` little-endian length, then bytes) messages. The host
//! holds a link to every client and `broadcast` reaches all of them, so the
//! lockstep driver's host relay forwards each client's frames to the others. I/O
//! runs on a `tokio` runtime on a background thread, driven through `SocketIo`.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as TokioStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::oneshot;

use super::error::TransportError;
use super::socket_io::{ObservedAddrs, SharedState, SocketIo};
use super::{ConnectionState, NetworkTransport, TransportEvent};
use crate::peer::{HOST_PEER, PeerId};

/// An established set of connections: `(peer id, stream)` per remote.
type Connected = Vec<(PeerId, TokioStream)>;

/// One endpoint of a TCP star.
pub struct TcpTransport(SocketIo);

impl TcpTransport {
    /// Binds `addr` as the host (peer `0`) and accepts clients continuously in the
    /// background, assigning each an ascending [`PeerId`] as it connects. Returns
    /// immediately with no clients; each join surfaces as a
    /// [`PeerConnected`](TransportEvent::PeerConnected) from
    /// [`poll`](NetworkTransport::poll), and its source address is recorded for
    /// later lookup.
    pub fn host_open(addr: impl ToSocketAddrs) -> crate::transport::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;

        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();
        let state = SharedState::default();
        let task_state = state.clone();
        let observed = ObservedAddrs::default();
        let task_observed = Arc::clone(&observed);

        let thread = std::thread::Builder::new()
            .name("ferrets-tcp-host".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build tcp host runtime");
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener)
                        .expect("tokio tcp listener from_std");
                    serve_host(listener, outbound_rx, inbound_tx, task_state, task_observed).await;
                });
            })?;

        Ok(Self(SocketIo::new(
            HOST_PEER,
            Vec::new(),
            outbound_tx,
            inbound_rx,
            state,
            observed,
            thread,
        )))
    }

    /// Connects to a host and blocks until it has assigned this client's peer id.
    pub fn join(addr: impl ToSocketAddrs) -> crate::transport::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true).ok();
        stream.set_nonblocking(true)?;
        // Record where the host was actually reached: an address a peer
        // advertises about itself can be its unroutable bind address, and the
        // one this connect used is the proven-reachable substitute.
        let observed = ObservedAddrs::default();
        observed
            .lock()
            .expect("observed-address map is never poisoned")
            .insert(HOST_PEER, stream.peer_addr()?);
        Self::spawn(observed, move || join_host(stream))
    }

    /// Builds one node of a full TCP mesh over an already-bound `listener`:
    /// dials every peer in `dial` (announcing `local` as an 8-byte prelude) and
    /// accepts one inbound link from each peer id in `accept` (learning each
    /// dialer's id from its prelude, and rejecting a prelude that is not an
    /// expected id or repeats one already linked). By convention the lower peer
    /// id dials, so `dial` holds the higher ids and `accept` the lower ones.
    ///
    /// Returns without waiting for the links: they form on the background
    /// runtime, and [`state`](NetworkTransport::state) reports
    /// [`Connecting`](ConnectionState::Connecting) until every link is up (then
    /// [`Connected`](ConnectionState::Connected)) or the attempt times out
    /// ([`Disconnected`](ConnectionState::Disconnected)). Messages broadcast
    /// before the mesh is up are queued and flushed once it is. Not blocking
    /// keeps a caller on a UI thread responsive while the mesh converges.
    pub fn mesh(
        local: PeerId,
        listener: TcpListener,
        dial: Vec<(PeerId, SocketAddr)>,
        accept: Vec<PeerId>,
    ) -> crate::transport::Result<Self> {
        listener.set_nonblocking(true)?;
        let peers: Vec<PeerId> = dial
            .iter()
            .map(|(peer, _)| *peer)
            .chain(accept.iter().copied())
            .collect();

        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let state = SharedState::default();
        let task_state = state.clone();

        let thread = std::thread::Builder::new()
            .name("ferrets-tcp-mesh".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build tcp mesh runtime");
                runtime.block_on(async move {
                    tokio::select! {
                        // The handle was dropped before the links came up: stop
                        // waiting rather than hold teardown for the timeout.
                        _ = cancel_rx => task_state.set(ConnectionState::Disconnected),
                        result = mesh_connect(local, listener, dial, accept) => match result {
                            Ok((_, conns)) => {
                                serve(conns, outbound_rx, inbound_tx, task_state).await;
                            }
                            Err(error) => {
                                eprintln!("tcp mesh links did not come up: {error}");
                                task_state.set(ConnectionState::Disconnected);
                            }
                        },
                    }
                });
            })?;

        let mut io = SocketIo::new(
            local,
            peers,
            outbound_tx,
            inbound_rx,
            state,
            ObservedAddrs::default(),
            thread,
        );
        io.set_cancel(cancel_tx);
        Ok(Self(io))
    }

    /// Spawns the background runtime. `connect` runs first and yields this node's
    /// own [`PeerId`] plus its connections; once it resolves, the serve loop runs
    /// and the constructor returns a fully-mapped transport.
    fn spawn<F, Fut>(observed: ObservedAddrs, connect: F) -> crate::transport::Result<Self>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = crate::transport::Result<(PeerId, Connected)>>,
    {
        let (ready_tx, ready_rx) =
            std::sync::mpsc::sync_channel::<crate::transport::Result<(PeerId, Vec<PeerId>)>>(1);
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded();
        let state = SharedState::default();
        let task_state = state.clone();

        let thread = std::thread::Builder::new()
            .name("ferrets-tcp".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build tcp runtime");
                runtime.block_on(async move {
                    match connect().await {
                        Ok((local, conns)) => {
                            let peers = conns.iter().map(|(id, _)| *id).collect();
                            let _ = ready_tx.send(Ok((local, peers)));
                            serve(conns, outbound_rx, inbound_tx, task_state).await;
                        }
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                        }
                    }
                })
            })?;

        let (local, peers) = ready_rx
            .recv()
            .map_err(|_| io::Error::other("io thread died during connect"))??;

        Ok(Self(SocketIo::new(
            local,
            peers,
            outbound_tx,
            inbound_rx,
            state,
            observed,
            thread,
        )))
    }
}

/// How long a mesh node waits for all of its links before giving up.
const MESH_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Mesh connect: dial the higher peer ids, then accept the lower ones.
async fn mesh_connect(
    local: PeerId,
    listener: TcpListener,
    dial: Vec<(PeerId, SocketAddr)>,
    accept: Vec<PeerId>,
) -> crate::transport::Result<(PeerId, Connected)> {
    let links = async move {
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let mut conns = Vec::with_capacity(dial.len() + accept.len());
        for (peer, addr) in dial {
            let mut stream = TokioStream::connect(addr).await?;
            stream.set_nodelay(true).ok();
            stream.write_all(&local.to_le_bytes()).await?;
            conns.push((peer, stream));
        }
        // A dialer announces its own id; trust it only if it is one we expect
        // and have not already linked, so a duplicate or forged prelude cannot
        // impersonate another peer's link or collide in the connection set.
        let mut expected: BTreeSet<PeerId> = accept.into_iter().collect();
        while !expected.is_empty() {
            let (mut stream, _) = listener.accept().await?;
            stream.set_nodelay(true).ok();
            let mut id_bytes = [0u8; 8];
            stream.read_exact(&mut id_bytes).await?;
            let peer = u64::from_le_bytes(id_bytes);
            if !expected.remove(&peer) {
                return Err(TransportError::InternalError(format!(
                    "mesh accepted an unexpected or duplicate peer id {peer}"
                )));
            }
            conns.push((peer, stream));
        }
        Ok((local, conns))
    };
    tokio::time::timeout(MESH_CONNECT_TIMEOUT, links)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "mesh links did not come up"))?
}

/// Client connect: read the assigned id from the host.
async fn join_host(stream: TcpStream) -> crate::transport::Result<(PeerId, Connected)> {
    let mut stream = TokioStream::from_std(stream)?;
    let mut id_bytes = [0u8; 8];
    stream.read_exact(&mut id_bytes).await?;
    Ok((u64::from_le_bytes(id_bytes), vec![(HOST_PEER, stream)]))
}

/// The serve loop: spawn a reader per connection, then fan `broadcast`s out to
/// every connection's writer. Exits when the outbound channel closes.
async fn serve(
    conns: Connected,
    mut outbound: UnboundedReceiver<Vec<u8>>,
    inbound: crossbeam_channel::Sender<TransportEvent>,
    state: SharedState,
) {
    let mut writers = Vec::with_capacity(conns.len());
    for (id, stream) in conns {
        let (read, write) = stream.into_split();
        writers.push(write);
        tokio::spawn(read_loop(id, read, inbound.clone()));
        let _ = inbound.send(TransportEvent::PeerConnected(id));
    }
    state.set(ConnectionState::Connected);

    while let Some(bytes) = outbound.recv().await {
        let len = (bytes.len() as u32).to_le_bytes();
        for writer in &mut writers {
            let _ = writer.write_all(&len).await;
            let _ = writer.write_all(&bytes).await;
        }
    }

    state.set(ConnectionState::Disconnected);
}

/// The continuously-accepting host serve loop: accept clients forever (assigning
/// ascending ids and recording each one's source address), and fan `broadcast`s
/// out to every live connection, dropping any whose write fails. Exits when the
/// outbound channel closes.
async fn serve_host(
    listener: tokio::net::TcpListener,
    mut outbound: UnboundedReceiver<Vec<u8>>,
    inbound: crossbeam_channel::Sender<TransportEvent>,
    state: SharedState,
    observed: ObservedAddrs,
) {
    state.set(ConnectionState::Connected);
    let mut next_id: PeerId = HOST_PEER + 1;
    // BTreeMap so the broadcast fan-out order is deterministic (by peer id).
    let mut writers: BTreeMap<PeerId, OwnedWriteHalf> = BTreeMap::new();

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((mut stream, src)) = accepted else { continue };
                stream.set_nodelay(true).ok();
                let id = next_id;
                if stream.write_all(&id.to_le_bytes()).await.is_err() {
                    continue;
                }
                next_id += 1;
                observed.lock().unwrap().insert(id, src);
                let (read, write) = stream.into_split();
                writers.insert(id, write);
                tokio::spawn(read_loop(id, read, inbound.clone()));
                let _ = inbound.send(TransportEvent::PeerConnected(id));
            }
            outgoing = outbound.recv() => {
                let Some(bytes) = outgoing else { break };
                let len = (bytes.len() as u32).to_le_bytes();
                let mut dead = Vec::new();
                for (&id, writer) in writers.iter_mut() {
                    if writer.write_all(&len).await.is_err()
                        || writer.write_all(&bytes).await.is_err()
                    {
                        dead.push(id);
                    }
                }
                for id in dead {
                    writers.remove(&id);
                }
            }
        }
    }

    state.set(ConnectionState::Disconnected);
}

/// Reads length-prefixed messages from one peer until the link closes.
async fn read_loop(
    id: PeerId,
    mut read: OwnedReadHalf,
    inbound: crossbeam_channel::Sender<TransportEvent>,
) {
    loop {
        let mut len_bytes = [0u8; 4];
        if read.read_exact(&mut len_bytes).await.is_err() {
            break;
        }
        let len = u32::from_le_bytes(len_bytes) as usize;
        let mut bytes = vec![0u8; len];
        if read.read_exact(&mut bytes).await.is_err() {
            break;
        }
        if inbound
            .send(TransportEvent::Message { from: id, bytes })
            .is_err()
        {
            return;
        }
    }
    let _ = inbound.send(TransportEvent::PeerDisconnected(id));
}

impl NetworkTransport for TcpTransport {
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

    fn observed_addr(&self, peer: PeerId) -> Option<std::net::SocketAddr> {
        self.0.observed_addr(peer)
    }
}
