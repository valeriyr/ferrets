//! Building a [`NetSession`] when the lobby starts a host-star game, over the
//! in-process loopback transport.

use ferrets_network::control::ControlChannel;
use ferrets_network::lobby::client::LobbyClient;
use ferrets_network::lobby::host::LobbyHost;
use ferrets_network::message::control::{ControlMessage, InGameMessage, Occupant};
use ferrets_network::session::NetSession;
use ferrets_network::topology::Topology;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_simulation::session::ai_hosting::AiHosting;

//
// ─── Host-star start ──────────────────────────────────────────────────────────
//

#[test]
fn host_star_start_builds_gameplay_channel_mapped_from_slots() {
    let mut endpoints = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)]).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let host = LobbyHost::new(
        ControlChannel::new(Box::new(ep0)),
        Topology::HostStar,
        AiHosting::Replicated,
        3,
        "human",
    );

    let mut host = host;
    host.poll().expect("seat the two connected clients");

    let bind = "127.0.0.1:0".parse().expect("addr");
    let mut session = NetSession::start_host(host, bind).expect("start host");

    let gameplay = session.gameplay();
    assert_eq!(gameplay.local_player(), 0);
    assert_eq!(gameplay.player_count(), 3);
    assert!(gameplay.is_networked(1));
    assert!(gameplay.is_networked(2));
    // The host is the control-plane host in every topology.
    assert!(session.is_control_host());
}

#[test]
fn ai_slots_are_networked_only_under_host_only_hosting() {
    for (mode, networked) in [(AiHosting::Replicated, false), (AiHosting::HostOnly, true)] {
        let mut endpoints = LoopbackTransport::partial_mesh(2, [(0, 1)]).into_iter();
        let ep0 = endpoints.next().expect("host endpoint");
        let mut host = LobbyHost::new(
            ControlChannel::new(Box::new(ep0)),
            Topology::HostStar,
            mode,
            3,
            "human",
        );
        host.poll().expect("seat the connected client");
        host.set_occupant(2, Occupant::Ai).expect("slot 2 is an ai");

        let bind = "127.0.0.1:0".parse().expect("addr");
        let mut session = NetSession::start_host(host, bind).expect("start host");

        assert_eq!(session.gameplay().is_networked(2), networked, "{mode:?}");
        assert!(session.gameplay().is_networked(1), "{mode:?}");
    }
}

//
// ─── In-game control over a shared host-star socket ────────────────────────────
//

#[test]
fn control_flows_both_ways_after_host_star_game_starts() {
    let mut endpoints = LoopbackTransport::partial_mesh(2, [(0, 1)]).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let ep1 = endpoints.next().expect("client endpoint");

    let mut host = LobbyHost::new(
        ControlChannel::new(Box::new(ep0)),
        Topology::HostStar,
        AiHosting::Replicated,
        2,
        "human",
    );
    let mut client = LobbyClient::new(ControlChannel::new(Box::new(ep1)));

    host.poll().expect("seat the client");
    client.poll();
    let bind = "127.0.0.1:0".parse().expect("addr");
    let mut host = NetSession::start_host(host, bind).expect("start host");
    client.poll(); // receive the host's start signal
    let mut client = NetSession::start_client(client, bind).expect("start client");

    // Client → host: a pause request reaches the host over the shared socket.
    client
        .send_control(&ControlMessage::InGame(InGameMessage::PauseRequest {
            paused: true,
        }))
        .expect("client sends");
    host.drain_received(); // fills the control buffer from the shared socket
    assert_eq!(
        host.drain_control(),
        vec![ControlMessage::InGame(InGameMessage::PauseRequest {
            paused: true
        })],
    );

    // Host → client: the authoritative pause reaches the client.
    host.send_control(&ControlMessage::InGame(InGameMessage::PauseAt {
        tick: 50,
        paused: true,
    }))
    .expect("host sends");
    client.drain_received();
    assert_eq!(
        client.drain_control(),
        vec![ControlMessage::InGame(InGameMessage::PauseAt {
            tick: 50,
            paused: true
        })],
    );
}
