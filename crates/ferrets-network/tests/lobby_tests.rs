//! The host-coordinated lobby state machine, driven over the in-process loopback
//! transport (host = endpoint 0, clients linked only to the host).

mod utils;

use ferrets_network::control::{ControlChannel, ControlEvent};
use ferrets_network::error::NetworkError;
use ferrets_network::lobby::client::{LobbyClient, PollOutcome};
use ferrets_network::lobby::host::LobbyHost;
use ferrets_network::message::control::{ControlMessage, LobbyMessage, Occupant};
use ferrets_network::session_mode::SessionMode;
use ferrets_network::transport::error::TransportError;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_simulation::session::ai_hosting::AiHosting;

//
// ─── Seating and live sync ──────────────────────────────────────────────────
//

#[test]
fn host_seats_connecting_clients_in_open_slots() {
    let (mut host, _c1, _c2) = star(3);

    let changed = host.poll().expect("host poll");

    assert!(changed);
    assert_eq!(occupants(&host), vec![human(0), human(1), human(2)]);
}

#[test]
fn clients_mirror_broadcast_state_and_find_own_slot() {
    let (mut host, mut c1, mut c2) = star(3);

    host.poll().expect("host poll");
    c1.poll();
    c2.poll();

    assert_eq!(c1.local_player(), Some(1));
    assert_eq!(c2.local_player(), Some(2));
    assert_eq!(
        c1.state().map(|s| s.mode),
        Some(SessionMode::HostStar {
            ai_hosting: AiHosting::Replicated
        }),
    );
    assert_eq!(c1.slots().len(), 3);
}

#[test]
fn client_race_request_updates_every_node() {
    let (mut host, mut c1, mut c2) = star(3);
    host.poll().expect("seat");
    c1.poll();
    c2.poll();

    c1.request_race("orc").expect("request race");
    let changed = host.poll().expect("host applies request");
    c1.poll();
    c2.poll();

    assert!(changed);
    assert_eq!(host.slots()[1].race.as_deref(), Some("orc"));
    assert_eq!(c2.slots()[1].race.as_deref(), Some("orc"));
}

#[test]
fn client_team_request_updates_every_node() {
    let (mut host, mut c1, mut c2) = star(3);
    host.poll().expect("seat");
    c1.poll();
    c2.poll();

    c1.request_team(Some(2)).expect("request team");
    let changed = host.poll().expect("host applies request");
    c1.poll();
    c2.poll();

    assert!(changed);
    assert_eq!(host.slots()[1].team, Some(2));
    assert_eq!(c2.slots()[1].team, Some(2));
}

#[test]
fn host_sets_a_slots_team_for_every_node() {
    let (mut host, _c1, mut c2) = star(3);
    host.poll().expect("seat");
    c2.poll();

    // The host arranges an AI slot's team; a client sees it on the next poll.
    host.set_team(2, Some(1)).expect("host sets team");
    c2.poll();

    assert_eq!(host.slots()[2].team, Some(1));
    assert_eq!(c2.slots()[2].team, Some(1));
}

#[test]
fn host_keeps_client_on_matching_version() {
    let (mut host, mut c1, _c2) = star(3);
    host.poll().expect("seat on connect");
    c1.poll();

    // join() sends this build's PROTOCOL_VERSION, which matches the host.
    c1.join(None, None).expect("join");
    host.poll().expect("host applies join");

    assert_eq!(host.slots()[1].occupant, human(1));
    // The client keeps its seat: the host re-broadcasts state rather than refusing.
    assert!(matches!(c1.poll(), PollOutcome::Waiting { .. }));
}

#[test]
fn host_rejects_client_on_version_mismatch() {
    let mut endpoints = LoopbackTransport::partial_mesh(2, [(0, 1)]).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let ep1 = endpoints.next().expect("client endpoint");
    let mut host = utils::lobby_host(
        ep0,
        SessionMode::HostStar {
            ai_hosting: AiHosting::Replicated,
        },
        2,
    );
    let mut client = ControlChannel::new(Box::new(ep1));

    host.poll().expect("seat the client on connect");
    client
        .send(&ControlMessage::Lobby(LobbyMessage::Join {
            protocol_version: "99.0".to_string(),
            advertised_udp_port: None,
            advertised_control_port: None,
            race: None,
        }))
        .expect("send mismatched join");
    host.poll().expect("host processes the join");

    // The slot the client briefly held is reopened, and it is told why.
    assert_eq!(host.slots()[1].occupant, Occupant::Open);
    let rejected = client.poll().into_iter().any(|event| {
        matches!(
            event,
            ControlEvent::Message {
                message: ControlMessage::Lobby(LobbyMessage::Rejected { reason, .. }),
                ..
            } if reason.contains("build mismatch")
        )
    });
    assert!(rejected);
}

#[test]
fn clients_mirror_ai_hosting_changes() {
    let (mut host, mut c1, _c2) = star(3);
    host.poll().expect("seat two clients");
    c1.poll();

    assert_eq!(
        c1.state().map(|s| s.mode.ai_hosting()),
        Some(AiHosting::Replicated)
    );

    host.set_mode(SessionMode::HostStar {
        ai_hosting: AiHosting::Host,
    })
    .expect("set the mode");
    c1.poll();

    assert_eq!(
        c1.state().map(|s| s.mode.ai_hosting()),
        Some(AiHosting::Host)
    );
}

#[test]
fn join_fails_when_requested_udp_port_is_taken() {
    // A player-configured port must be used exactly or fail loudly — never
    // silently substituted, or their firewall setup stops matching reality.
    let taken = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).expect("bind");
    let port = taken.local_addr().expect("addr").port();
    let (mut host, mut c1, _c2) = star(3);
    host.poll().expect("seat two clients");
    c1.poll();

    let Err(error) = c1.join(Some(port), None) else {
        panic!("must reject the occupied port");
    };

    assert!(
        matches!(
            &error,
            NetworkError::TransportError(TransportError::IoError(io))
                if io.kind() == std::io::ErrorKind::AddrInUse
        ),
        "got {error:?}"
    );
}

#[test]
fn host_can_close_open_slot() {
    let (mut host, _c1, _c2) = star(4);
    host.poll().expect("seat two clients");

    // Slot 3 stayed open; the host closes it.
    host.set_occupant(3, Occupant::Closed).expect("close");

    assert_eq!(host.slots()[3].occupant, Occupant::Closed);
    assert_eq!(host.slots()[1].occupant, human(1));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Builds a host with `capacity` slots and two clients linked only to the host.
fn star(capacity: usize) -> (LobbyHost, LobbyClient, LobbyClient) {
    let mut endpoints = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)]).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let ep1 = endpoints.next().expect("client 1 endpoint");
    let ep2 = endpoints.next().expect("client 2 endpoint");

    let host = utils::lobby_host(
        ep0,
        SessionMode::HostStar {
            ai_hosting: AiHosting::Replicated,
        },
        capacity,
    );
    (host, utils::lobby_client(ep1), utils::lobby_client(ep2))
}

fn human(peer: u64) -> Occupant {
    Occupant::Human { peer }
}

fn occupants(host: &LobbyHost) -> Vec<Occupant> {
    host.slots().iter().map(|info| info.occupant).collect()
}
