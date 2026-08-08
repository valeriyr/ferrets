//! The TCP star transport: a host and a client exchanging messages over a real
//! localhost socket.

mod utils;

use std::{
    net::TcpListener,
    time::{Duration, Instant},
};

use ferrets_network::transport::{NetworkTransport, TransportEvent, tcp::TcpTransport};

//
// ─── Localhost exchange ─────────────────────────────────────────────────────
//

#[test]
fn host_and_client_exchange_messages_both_ways() {
    // Scout a free port, then host on it; the client retries until the host binds.
    let scout = TcpListener::bind("127.0.0.1:0").expect("scout bind");
    let addr = scout.local_addr().expect("addr");
    drop(scout);

    let mut host = TcpTransport::host_open(addr).expect("host_open");
    let mut client = join_retrying(addr);
    assert_eq!(host.local_peer(), 0);
    assert_eq!(client.local_peer(), 1);
    assert_eq!(poll_for_connect(&mut host), 1);

    host.broadcast(&[10, 20, 30]).expect("host send");
    assert_eq!(poll_for_message(&mut client), (0, vec![10, 20, 30]));

    client.broadcast(&[40, 50]).expect("client send");
    assert_eq!(poll_for_message(&mut host), (1, vec![40, 50]));
}

//
// ─── Continuous accept ────────────────────────────────────────────────────────
//

#[test]
fn open_host_accepts_clients_live_with_ascending_ids() {
    let scout = TcpListener::bind("127.0.0.1:0").expect("scout bind");
    let addr = scout.local_addr().expect("addr");
    drop(scout);

    // The host returns immediately with no clients and accepts as they arrive.
    let mut host = TcpTransport::host_open(addr).expect("host_open");
    assert_eq!(host.local_peer(), 0);

    let mut first = join_retrying(addr);
    assert_eq!(first.local_peer(), 1);
    assert_eq!(poll_for_connect(&mut host), 1);

    // A second client joins the already-running host and gets the next id.
    let mut second = join_retrying(addr);
    assert_eq!(second.local_peer(), 2);
    assert_eq!(poll_for_connect(&mut host), 2);

    // Broadcasts reach every connected client.
    host.broadcast(&[7, 7]).expect("host send");
    assert_eq!(poll_for_message(&mut first), (0, vec![7, 7]));
    assert_eq!(poll_for_message(&mut second), (0, vec![7, 7]));

    // The host recorded each client's source address.
    assert!(host.observed_addr(1).is_some());
    assert!(host.observed_addr(2).is_some());
}

//
// ─── Teardown ─────────────────────────────────────────────────────────────────
//

#[test]
fn dropping_connecting_mesh_does_not_wait_out_timeout() {
    // A mesh node awaiting an inbound link that never comes would otherwise hold
    // its teardown for the whole connect timeout; the cancellation makes the
    // drop return at once.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let transport = TcpTransport::mesh(0, listener, Vec::new(), vec![1]).expect("mesh");

    let start = Instant::now();
    drop(transport);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "drop waited out the connect timeout ({:?})",
        start.elapsed(),
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Joins `addr`, retrying until the host's listener is up.
fn join_retrying(addr: std::net::SocketAddr) -> TcpTransport {
    utils::wait_for("host listener to accept", || TcpTransport::join(addr).ok())
}

/// Blocks until a `PeerConnected` event arrives, returning the new peer id.
fn poll_for_connect(transport: &mut TcpTransport) -> u64 {
    utils::wait_for("peer to connect", || {
        transport.poll().into_iter().find_map(|event| match event {
            TransportEvent::PeerConnected(peer) => Some(peer),
            _ => None,
        })
    })
}

/// Blocks until a message event arrives, returning `(from, bytes)`.
fn poll_for_message(transport: &mut TcpTransport) -> (u64, Vec<u8>) {
    utils::wait_for("message to arrive", || {
        transport.poll().into_iter().find_map(|event| match event {
            TransportEvent::Message { from, bytes } => Some((from, bytes)),
            _ => None,
        })
    })
}
