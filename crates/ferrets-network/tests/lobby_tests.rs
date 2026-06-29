//! The host-coordinated lobby state machine, driven over the in-process loopback
//! transport (host = endpoint 0, clients linked only to the host).

use ferrets_network::control::ControlChannel;
use ferrets_network::lobby::client::LobbyClient;
use ferrets_network::lobby::host::LobbyHost;
use ferrets_network::message::control::Occupant;
use ferrets_network::topology::Topology;
use ferrets_network::transport::loopback::LoopbackTransport;

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
    assert_eq!(c1.topology(), Topology::HostStar);
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

    let host = LobbyHost::new(
        ControlChannel::new(Box::new(ep0)),
        Topology::HostStar,
        capacity,
        "human",
    );
    let c1 = LobbyClient::new(ControlChannel::new(Box::new(ep1)));
    let c2 = LobbyClient::new(ControlChannel::new(Box::new(ep2)));
    (host, c1, c2)
}

fn human(peer: u64) -> Occupant {
    Occupant::Human { peer }
}

fn occupants(host: &LobbyHost) -> Vec<Occupant> {
    host.slots().iter().map(|info| info.occupant).collect()
}
