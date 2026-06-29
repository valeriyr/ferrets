//! Shared async ↔ sync bridge for socket-backed transports.
//!
//! A socket transport runs its I/O on a `tokio` runtime on a background thread.
//! This holds the synchronous ends of that bridge — an outbound channel the sim
//! thread pushes bytes into and an inbound channel it drains events from — and
//! implements the synchronous [`NetworkTransport`](super::NetworkTransport) surface
//! over them. The async task (UDP datagram loop, TCP connection tasks) is supplied
//! by each transport.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::Receiver;
use tokio::sync::mpsc::UnboundedSender;

use super::{ConnectionState, TransportEvent};
use crate::peer::PeerId;
use crate::transport::error::TransportError;

/// Per-peer source addresses a background task observed on connect, shared with
/// the synchronous handle. Empty for transports that do not track them.
pub(crate) type ObservedAddrs = Arc<Mutex<HashMap<PeerId, SocketAddr>>>;

/// Connection state shared with the background task, encoded for atomic access.
#[derive(Clone, Default)]
pub(crate) struct SharedState(Arc<AtomicU8>);

impl SharedState {
    const CONNECTING: u8 = 0;
    const CONNECTED: u8 = 1;
    const DISCONNECTED: u8 = 2;

    pub(crate) fn set(&self, state: ConnectionState) {
        let value = match state {
            ConnectionState::Connecting => Self::CONNECTING,
            ConnectionState::Connected => Self::CONNECTED,
            ConnectionState::Disconnected => Self::DISCONNECTED,
        };
        self.0.store(value, Ordering::Relaxed);
    }

    fn get(&self) -> ConnectionState {
        match self.0.load(Ordering::Relaxed) {
            Self::CONNECTED => ConnectionState::Connected,
            Self::DISCONNECTED => ConnectionState::Disconnected,
            _ => ConnectionState::Connecting,
        }
    }
}

/// The synchronous handle to a background socket I/O task.
pub(crate) struct SocketIo {
    local: PeerId,
    peers: Vec<PeerId>,
    /// Bytes to broadcast, sent to the I/O task. `None` once dropped (shutdown).
    outbound: Option<UnboundedSender<Vec<u8>>>,
    inbound: Receiver<TransportEvent>,
    state: SharedState,
    observed: ObservedAddrs,
    thread: Option<JoinHandle<()>>,
}

impl SocketIo {
    pub(crate) fn new(
        local: PeerId,
        peers: Vec<PeerId>,
        outbound: UnboundedSender<Vec<u8>>,
        inbound: Receiver<TransportEvent>,
        state: SharedState,
        observed: ObservedAddrs,
        thread: JoinHandle<()>,
    ) -> Self {
        Self {
            local,
            peers,
            outbound: Some(outbound),
            inbound,
            state,
            observed,
            thread: Some(thread),
        }
    }

    pub(crate) fn local_peer(&self) -> PeerId {
        self.local
    }

    pub(crate) fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    /// The source address observed for `peer` when it connected, if tracked.
    pub(crate) fn observed_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        self.observed.lock().unwrap().get(&peer).copied()
    }

    pub(crate) fn broadcast(&mut self, bytes: &[u8]) -> crate::transport::Result<()> {
        let sender = self
            .outbound
            .as_ref()
            .ok_or_else(|| TransportError::InternalError("io task stopped".into()))?;
        sender
            .send(bytes.to_vec())
            .map_err(|_| TransportError::InternalError("io task stopped".into()))
    }

    pub(crate) fn poll(&mut self) -> Vec<TransportEvent> {
        self.inbound.try_iter().collect()
    }

    pub(crate) fn state(&self) -> ConnectionState {
        self.state.get()
    }
}

impl Drop for SocketIo {
    fn drop(&mut self) {
        // Dropping the only outbound sender closes the channel; the I/O task's
        // `recv` then returns `None` and the task exits. Join so the runtime and
        // socket are torn down before we return.
        self.outbound = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
