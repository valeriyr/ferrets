//! The lockstep driver: broadcasting and decoding frames and checksums over a
//! transport, with peer-to-slot translation.

use ferrets_network::driver::{LockstepDriver, PeerChecksum};
use ferrets_network::role::Role;
use ferrets_network::roster::Roster;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::input::PlayerFrame;

//
// ─── Frame exchange ─────────────────────────────────────────────────────────
//

#[test]
fn broadcast_frames_reach_other_peer() {
    let (mut host, mut peer) = connected_pair();
    assert_eq!(host.local_player(), 0);
    assert_eq!(peer.local_player(), 1);

    let frame = PlayerFrame {
        player: host.local_player(),
        tick: 5,
        commands: vec![PlayerCommand::Stop],
    };
    host.broadcast_frames(vec![frame]).expect("broadcast");

    let received = peer.drain_received();
    assert_eq!(received.frames.len(), 1);

    let got = &received.frames[0];

    assert_eq!(got.player, 0); // originator, carried in the payload
    assert_eq!(got.tick, 5);
    assert_eq!(got.commands, vec![PlayerCommand::Stop]);
}

//
// ─── Checksum exchange ──────────────────────────────────────────────────────
//

#[test]
fn checksum_round_trips() {
    let (mut host, mut peer) = connected_pair();

    host.send_checksum(8, 0xDEAD_BEEF).expect("send");

    let received = peer.drain_received();
    assert_eq!(
        received.checksums,
        vec![PeerChecksum {
            player: 0,
            tick: 8,
            hash: 0xDEAD_BEEF,
        }],
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Two drivers over a loopback pair, with the initial connection drained so
/// only message traffic remains.
fn connected_pair() -> (LockstepDriver, LockstepDriver) {
    let (a, b) = LoopbackTransport::pair();
    // The two-peer roster as a lobby would have assigned it: peer 0, peer 1.
    let mut host = LockstepDriver::new(Box::new(a), Role::Peer, Roster::new(vec![0, 1]));
    let mut peer = LockstepDriver::new(Box::new(b), Role::Peer, Roster::new(vec![0, 1]));
    // First drain registers the connection both ways.
    host.drain_received();
    peer.drain_received();
    (host, peer)
}
