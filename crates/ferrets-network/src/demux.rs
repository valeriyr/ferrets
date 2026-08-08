//! Splitting one transport into a control view and a gameplay view.
//!
//! A host-star game runs both channels over a single socket. This wraps that one
//! transport and routes each message to the right view by its [`Message`] variant
//! (gameplay vs control), so the control channel and the gameplay driver each see
//! only their own traffic and neither needs to know about the other. Connection
//! events go to both.
//!
//! This sits above the transport layer (it understands the [`Message`] envelope)
//! while still presenting a [`NetworkTransport`] to each channel.

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use crate::{
    message::{self, Message},
    peer::PeerId,
    transport::{ConnectionState, NetworkTransport, TransportEvent},
};

/// One of the two planes a split socket carries.
#[derive(Clone, Copy)]
enum Plane {
    Control,
    Gameplay,
}

/// Splits `transport` into `(control, gameplay)` views that share the one socket.
/// Sends from either view go out as-is (already tagged); received messages are
/// routed by their [`Message`] variant, and connection events are delivered to both.
pub fn split(
    transport: Box<dyn NetworkTransport>,
) -> (Box<dyn NetworkTransport>, Box<dyn NetworkTransport>) {
    let shared = Arc::new(Mutex::new(Shared {
        inner: transport,
        control: VecDeque::new(),
        gameplay: VecDeque::new(),
    }));
    let control = Box::new(View {
        shared: Arc::clone(&shared),
        plane: Plane::Control,
    });
    let gameplay = Box::new(View {
        shared,
        plane: Plane::Gameplay,
    });
    (control, gameplay)
}

/// The shared socket and the per-channel inboxes the views drain.
struct Shared {
    inner: Box<dyn NetworkTransport>,
    control: VecDeque<TransportEvent>,
    gameplay: VecDeque<TransportEvent>,
}

impl Shared {
    /// Drains the socket once and routes each event to the channel inboxes.
    fn pump(&mut self) {
        for event in self.inner.poll() {
            match &event {
                TransportEvent::Message { bytes, .. } => match message::decode(bytes) {
                    Ok(Message::Control(_)) => self.control.push_back(event),
                    Ok(Message::Gameplay(_)) => self.gameplay.push_back(event),
                    Err(_) => {}
                },
                // A connect/disconnect concerns both channels.
                TransportEvent::PeerConnected(_) | TransportEvent::PeerDisconnected(_) => {
                    self.control.push_back(event.clone());
                    self.gameplay.push_back(event);
                }
            }
        }
    }
}

/// One channel's view of the shared socket.
struct View {
    shared: Arc<Mutex<Shared>>,
    plane: Plane,
}

impl NetworkTransport for View {
    fn local_peer(&self) -> PeerId {
        self.shared.lock().unwrap().inner.local_peer()
    }

    fn broadcast(&mut self, bytes: &[u8]) -> crate::transport::Result<()> {
        self.shared.lock().unwrap().inner.broadcast(bytes)
    }

    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut shared = self.shared.lock().unwrap();
        shared.pump();
        let inbox = match self.plane {
            Plane::Control => &mut shared.control,
            Plane::Gameplay => &mut shared.gameplay,
        };
        inbox.drain(..).collect()
    }

    fn peers(&self) -> &[PeerId] {
        // Membership is derived from drained events / the roster, never from here.
        &[]
    }

    fn state(&self) -> ConnectionState {
        self.shared.lock().unwrap().inner.state()
    }

    fn observed_addr(&self, peer: PeerId) -> Option<SocketAddr> {
        self.shared.lock().unwrap().inner.observed_addr(peer)
    }
}
