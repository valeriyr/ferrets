//! The in-process loopback transport: byte exchange between a connected pair.

use ferrets_network::transport::{NetworkTransport, TransportEvent, loopback::LoopbackTransport};

//
// ─── Byte exchange ──────────────────────────────────────────────────────────
//

#[test]
fn pair_exchanges_bytes_with_peer_ids() {
    let (mut a, mut b) = LoopbackTransport::pair();
    assert_eq!(a.local_peer(), 0);
    assert_eq!(b.local_peer(), 1);

    a.broadcast(&[1, 2, 3]).expect("broadcast");

    let events = b.poll();
    // First poll announces the peer, then delivers the message.
    assert!(matches!(events[0], TransportEvent::PeerConnected(0)));
    match &events[1] {
        TransportEvent::Message { from, bytes } => {
            assert_eq!(*from, 0);
            assert_eq!(bytes, &[1, 2, 3]);
        }
        other => panic!("expected a message, got {other:?}"),
    }
}
