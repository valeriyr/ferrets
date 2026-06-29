//! The node's session role and its relay policy.

use ferrets_network::role::Role;

//
// ─── Relay policy ─────────────────────────────────────────────────────────────
//

#[test]
fn only_peer_and_host_relay() {
    assert!(Role::Peer.relays());
    assert!(Role::Host.relays());
    assert!(!Role::Client.relays());
}
