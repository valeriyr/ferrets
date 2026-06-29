//! Lockstep P2P networking for deterministic multiplayer.
//!
//! Peers exchange [`Message`](crate::message::Message)s (player frames, desync
//! checks) over a [`NetworkTransport`](crate::transport::NetworkTransport). The
//! transport-agnostic driver feeds remote frames into
//! the simulation's input queue and broadcasts the local player's frames, while
//! the simulation stays bit-deterministic.

pub mod bootstrap;
pub mod control;
pub mod demux;
pub mod driver;
pub mod error;
pub mod lobby;
pub mod message;
pub mod peer;
pub mod role;
pub mod roster;
pub mod session;
pub mod topology;
pub mod transport;

use crate::error::NetworkError;

/// The result type related to network.
pub type Result<T> = std::result::Result<T, NetworkError>;
