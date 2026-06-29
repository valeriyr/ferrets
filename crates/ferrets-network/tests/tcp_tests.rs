//! The TCP star transport: a host and a client exchanging messages over a real
//! localhost socket.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use ferrets_network::transport::tcp::TcpTransport;
use ferrets_network::transport::{NetworkTransport, TransportEvent};

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
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Joins `addr`, retrying until the host's listener is up.
fn join_retrying(addr: std::net::SocketAddr) -> TcpTransport {
    loop {
        match TcpTransport::join(addr) {
            Ok(client) => break client,
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Blocks until a `PeerConnected` event arrives, returning the new peer id.
fn poll_for_connect(transport: &mut TcpTransport) -> u64 {
    for _ in 0..200 {
        for event in transport.poll() {
            if let TransportEvent::PeerConnected(peer) = event {
                return peer;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("no peer connected within the time budget");
}

/// Blocks until a message event arrives, returning `(from, bytes)`, or panics
/// after a budget.
fn poll_for_message(transport: &mut TcpTransport) -> (u64, Vec<u8>) {
    for _ in 0..200 {
        for event in transport.poll() {
            if let TransportEvent::Message { from, bytes } = event {
                return (from, bytes);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("no message received within the time budget");
}
