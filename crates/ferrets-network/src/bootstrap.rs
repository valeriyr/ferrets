//! Opening or joining a lobby — the entry points a game calls to reach a
//! [`LobbyHost`] or [`LobbyClient`] over the reliable control transport (TCP).
//!
//! The chosen mode and policies decide only how the session is wired and
//! governed once the game starts (see
//! [`NetSession`](crate::session::NetSession)); the lobby itself is always
//! host-coordinated over TCP.

use std::net::ToSocketAddrs;

use crate::control::ControlChannel;
use crate::lobby::client::LobbyClient;
use crate::lobby::host::LobbyHost;
use crate::session_mode::SessionMode;
use crate::transport::tcp::TcpTransport;
use ferrets_simulation::session::drop_policy::DropPolicy;
use ferrets_simulation::session::finish_policy::FinishPolicy;

/// Opens a lobby as the host: binds `addr` and accepts clients continuously. The
/// lobby has `capacity` slots (slot `0` is the host) and new slots default to
/// `default_race`.
pub fn open_lobby(
    addr: impl ToSocketAddrs,
    mode: SessionMode,
    drop_policy: DropPolicy,
    finish_policy: FinishPolicy,
    capacity: usize,
    default_race: &str,
) -> crate::Result<LobbyHost> {
    let transport = TcpTransport::host_open(addr)?;
    let control = ControlChannel::new(Box::new(transport));
    Ok(LobbyHost::new(
        control,
        mode,
        drop_policy,
        finish_policy,
        capacity,
        default_race,
    ))
}

/// Joins a host's lobby as a client, blocking only until the host assigns this
/// client's peer id.
pub fn join_lobby(addr: impl ToSocketAddrs) -> crate::Result<LobbyClient> {
    let transport = TcpTransport::join(addr)?;
    let control = ControlChannel::new(Box::new(transport));
    Ok(LobbyClient::new(control))
}
