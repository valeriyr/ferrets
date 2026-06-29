//! Opening or joining a lobby — the entry points a game calls to reach a
//! [`LobbyHost`] or [`LobbyClient`] over the reliable control transport (TCP).
//!
//! The chosen [`Topology`] decides only how gameplay frames route once the game
//! starts (see [`NetSession`](crate::session::NetSession)); the lobby itself is
//! always host-coordinated over TCP.

use std::net::ToSocketAddrs;

use crate::control::ControlChannel;
use crate::lobby::client::LobbyClient;
use crate::lobby::host::LobbyHost;
use crate::topology::Topology;
use crate::transport::tcp::TcpTransport;

/// Opens a lobby as the host: binds `addr` and accepts clients continuously. The
/// lobby has `capacity` slots (slot `0` is the host) and new slots default to
/// `default_race`.
pub fn open_lobby(
    addr: impl ToSocketAddrs,
    topology: Topology,
    capacity: usize,
    default_race: &str,
) -> crate::Result<LobbyHost> {
    let transport = TcpTransport::host_open(addr)?;
    let control = ControlChannel::new(Box::new(transport));
    Ok(LobbyHost::new(control, topology, capacity, default_race))
}

/// Joins a host's lobby as a client, blocking only until the host assigns this
/// client's peer id.
pub fn join_lobby(addr: impl ToSocketAddrs) -> crate::Result<LobbyClient> {
    let transport = TcpTransport::join(addr)?;
    let control = ControlChannel::new(Box::new(transport));
    Ok(LobbyClient::new(control))
}
