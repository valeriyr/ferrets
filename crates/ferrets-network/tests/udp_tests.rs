//! The UDP transport: two endpoints exchanging datagrams over localhost.

mod utils;

use std::net::UdpSocket;

use ferrets_network::transport::udp::UdpTransport;
use ferrets_network::transport::{NetworkTransport, TransportEvent};

//
// ─── Localhost exchange ─────────────────────────────────────────────────────
//

#[test]
fn two_endpoints_exchange_datagrams_both_ways() {
    // Bind first so each side learns the other's ephemeral port before wiring up.
    let socket0 = UdpSocket::bind("127.0.0.1:0").expect("bind 0");
    let socket1 = UdpSocket::bind("127.0.0.1:0").expect("bind 1");
    let addr0 = socket0.local_addr().expect("addr 0");
    let addr1 = socket1.local_addr().expect("addr 1");

    let mut peer0 = UdpTransport::from_socket(0, socket0, vec![(1, addr1)]).expect("transport 0");
    let mut peer1 = UdpTransport::from_socket(1, socket1, vec![(0, addr0)]).expect("transport 1");

    peer0.broadcast(&[1, 2, 3]).expect("peer 0 send");
    assert_eq!(poll_for_message(&mut peer1), (0, vec![1, 2, 3]));

    peer1.broadcast(&[4, 5]).expect("peer 1 send");
    assert_eq!(poll_for_message(&mut peer0), (1, vec![4, 5]));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Blocks until a message event arrives, returning `(from, bytes)`, or panics
/// after a budget. UDP can drop, so the sender's redundancy normally handles
/// loss; on loopback a single send is reliable enough for the test.
fn poll_for_message(transport: &mut UdpTransport) -> (u64, Vec<u8>) {
    utils::wait_for("message to arrive", || {
        transport.poll().into_iter().find_map(|event| match event {
            TransportEvent::Message { from, bytes } => Some((from, bytes)),
            _ => None,
        })
    })
}
