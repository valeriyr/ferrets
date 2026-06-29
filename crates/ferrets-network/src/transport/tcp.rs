//! A TCP star transport: a host accepts clients, each client dials the host.
//!
//! The host is peer `0`; it assigns each client an ascending [`PeerId`] on
//! connect (sent as an 8-byte little-endian prelude) and thereafter exchanges
//! length-prefixed (`u32` little-endian length, then bytes) messages. The host
//! holds a link to every client and `broadcast` reaches all of them, so the
//! lockstep driver's host relay forwards each client's frames to the others. I/O
//! runs on a `tokio` runtime on a background thread, driven through `SocketIo`.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as TokioStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::socket_io::{ObservedAddrs, SharedState, SocketIo};
use super::{ConnectionState, NetworkTransport, TransportEvent};
use crate::peer::PeerId;

/// The host's peer id.
const HOST: PeerId = 0;

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
            HOST,
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
        Self::spawn(move || join_host(stream))
    }

    /// Spawns the background runtime. `connect` runs first and yields this node's
    /// own [`PeerId`] plus its connections; once it resolves, the serve loop runs
    /// and the constructor returns a fully-mapped transport.
    fn spawn<F, Fut>(connect: F) -> crate::transport::Result<Self>
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
            ObservedAddrs::default(),
            thread,
        )))
    }
}

/// Client connect: read the assigned id from the host.
async fn join_host(stream: TcpStream) -> crate::transport::Result<(PeerId, Connected)> {
    let mut stream = TokioStream::from_std(stream)?;
    let mut id_bytes = [0u8; 8];
    stream.read_exact(&mut id_bytes).await?;
    Ok((u64::from_le_bytes(id_bytes), vec![(HOST, stream)]))
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
    let mut next_id: PeerId = HOST + 1;
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
