//! Building a [`NetSession`] when the lobby starts a host-star game, over the
//! in-process loopback transport.

mod utils;

use ferrets_network::{
    bootstrap,
    message::control::{ControlMessage, InGameMessage, Occupant, Proposer},
    peer::{HOST_PEER, PeerId},
    session::NetSession,
    session_mode::SessionMode,
    transport::loopback::LoopbackTransport,
};
use ferrets_simulation::{
    input::PlayerFrame,
    session::{
        ai_hosting::AiHosting,
        ai_vision::AiVision,
        drop_policy::DropPolicy,
        elimination_scope::EliminationScope,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
};

/// The peer the host assigns to the one client that joins these two-node tests
/// (the host is [`HOST_PEER`]).
const CLIENT: PeerId = 1;

/// The player slot the host controls in these tests (the proposer of a
/// host-driven pause).
const HOST_PLAYER: PlayerId = 0;

//
// ─── Host-star start ──────────────────────────────────────────────────────────
//

#[test]
fn host_star_start_builds_gameplay_channel_mapped_from_slots() {
    let mut endpoints = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)]).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let host = utils::lobby_host(
        ep0,
        SessionMode::HostStar {
            ai_hosting: AiHosting::Replicated,
        },
        3,
    );

    let mut host = host;
    host.poll().expect("seat the two connected clients");

    let mut session = NetSession::start_host(host, None, &humans(3)).expect("start host");

    let gameplay = session.gameplay();
    assert_eq!(gameplay.local_player(), Some(0));
    assert_eq!(gameplay.player_count(), 3);
    assert!(gameplay.is_networked(1));
    assert!(gameplay.is_networked(2));
    // The host is the control-plane host in every topology.
    assert!(session.is_host_node());
}

#[test]
fn ai_slots_are_networked_only_under_host_only_hosting() {
    for (ai_hosting, networked) in [(AiHosting::Replicated, false), (AiHosting::Host, true)] {
        let mut endpoints = LoopbackTransport::partial_mesh(2, [(0, 1)]).into_iter();
        let ep0 = endpoints.next().expect("host endpoint");
        let mut host = utils::lobby_host(ep0, SessionMode::HostStar { ai_hosting }, 3);
        host.poll().expect("seat the connected client");
        host.set_occupant(2, Occupant::Ai).expect("slot 2 is an ai");

        let mut slots = humans(2);
        slots.push(PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            None,
            None,
        ));
        let mut session = NetSession::start_host(host, None, &slots).expect("start host");

        assert_eq!(
            session.gameplay().is_networked(2),
            networked,
            "{ai_hosting:?}"
        );
        assert!(session.gameplay().is_networked(1), "{ai_hosting:?}");
    }
}

#[test]
fn start_rejects_human_slot_without_connected_peer() {
    // The session claims two humans, but nobody joined the lobby: slot 1 has
    // no peer to feed its frames, and the error must say which seat disagrees.
    let mut endpoints = LoopbackTransport::partial_mesh(1, []).into_iter();
    let ep0 = endpoints.next().expect("host endpoint");
    let host = utils::lobby_host(
        ep0,
        SessionMode::HostStar {
            ai_hosting: AiHosting::Replicated,
        },
        2,
    );

    let Err(error) = NetSession::start_host(host, None, &humans(2)) else {
        panic!("must reject a human slot without a connected peer");
    };

    assert_eq!(
        error.to_string(),
        "transport error: internal error: human session slot 1 has no connected peer in the lobby",
    );
}

//
// ─── Mesh start over real sockets ─────────────────────────────────────────────
//

#[test]
fn decentralized_start_builds_control_mesh_outliving_lobby_star() {
    const TCP: u16 = 43119;

    let mut host = bootstrap::open_lobby(
        ("127.0.0.1", TCP),
        SessionMode::MeshDecentralized,
        DropPolicy::Automatic,
        FinishPolicy::LastStanding {
            elimination: EliminationScope::Player,
        },
        2,
        "human",
    )
    .expect("open lobby");
    let mut client = bootstrap::join_lobby(("127.0.0.1", TCP)).expect("join lobby");
    client.join(None, Some("orc")).expect("announce client");
    utils::wait_until("client is seated with its ports", || {
        host.poll().expect("host poll");
        host.client_udp_addr(CLIENT).is_some() && host.client_control_addr(CLIENT).is_some()
    });
    client.poll();

    // Starting drops the lobby star on both sides and links the peers'
    // control listeners directly; in-game control must flow over those links.
    // The host's side completes on its own: its dial lands in the client's
    // already-bound listener backlog.
    let mut host_session = NetSession::start_host(host, None, &humans(2)).expect("start host");
    utils::wait_until("client received the start signal", || {
        client.poll();
        client.started().is_some()
    });
    let mut client_session = NetSession::start_client(client, &humans(2)).expect("start client");

    let vote = InGameMessage::StallVote {
        voter: 1,
        tick: 4,
        missing: vec![0],
    };
    client_session
        .send_control(&ControlMessage::InGame(vote.clone()))
        .expect("client sends over the mesh");
    let mut received = Vec::new();
    utils::wait_until("host receives the vote over the mesh", || {
        received.extend(host_session.drain_control().messages);
        !received.is_empty()
    });
    // The host learns who sent it from the link itself, not the message body.
    assert_eq!(received, vec![(CLIENT, ControlMessage::InGame(vote))]);

    let pause = InGameMessage::PauseAt {
        proposer: Proposer::Player(HOST_PLAYER),
        tick: 12,
        paused: true,
    };
    host_session
        .send_control(&ControlMessage::InGame(pause.clone()))
        .expect("host sends over the mesh");
    let mut received = Vec::new();
    utils::wait_until("client receives the pause over the mesh", || {
        received.extend(client_session.drain_control().messages);
        !received.is_empty()
    });
    assert_eq!(received, vec![(HOST_PEER, ControlMessage::InGame(pause))]);
}

#[test]
fn mesh_start_exchanges_frames_both_ways_despite_unspecified_host_bind() {
    // Only the control (TCP) port must be fixed — the lobby is dialed by
    // address; both gameplay sockets bind ephemeral ports on their own.
    const TCP: u16 = 43117;

    let mut host = bootstrap::open_lobby(
        ("127.0.0.1", TCP),
        SessionMode::MeshHosted {
            ai_hosting: AiHosting::Host,
        },
        DropPolicy::Automatic,
        FinishPolicy::LastStanding {
            elimination: EliminationScope::Player,
        },
        2,
        "human",
    )
    .expect("open lobby");
    let mut client = bootstrap::join_lobby(("127.0.0.1", TCP)).expect("join lobby");
    client.join(None, Some("orc")).expect("announce client");
    utils::wait_until("client is seated with its port", || {
        host.poll().expect("host poll");
        host.client_udp_addr(CLIENT).is_some()
    });
    client.poll();

    // The host's gameplay socket binds the unspecified address — the
    // advertised table then carries `0.0.0.0`, which the client must resolve
    // to the address it reached the host at over the control channel.
    let mut host_session = NetSession::start_host(host, None, &humans(2)).expect("start host");
    utils::wait_until("client received the start signal", || {
        client.poll();
        client.started().is_some()
    });
    let mut client_session = NetSession::start_client(client, &humans(2)).expect("start client");

    // Host → client: a host-sourced frame (an AI's, under host-only hosting)
    // must reach the client even though the host advertised `0.0.0.0`.
    let ai_frame = PlayerFrame {
        player: 1,
        tick: 7,
        commands: Vec::new(),
    };
    let mut received = None;
    utils::wait_until("client receives the host's frame", || {
        host_session
            .broadcast_frames(vec![ai_frame.clone()])
            .expect("host broadcast");
        let frames = client_session.drain_received().frames;
        if let Some(frame) = frames.into_iter().next() {
            received = Some(frame);
            true
        } else {
            false
        }
    });
    assert_eq!(received.expect("frame"), ai_frame);

    // Client → host: the reply direction crosses the resolved address.
    let client_frame = PlayerFrame {
        player: 1,
        tick: 9,
        commands: Vec::new(),
    };
    let mut received = None;
    utils::wait_until("host receives the client's frame", || {
        client_session
            .broadcast_frames(vec![client_frame.clone()])
            .expect("client broadcast");
        let frames = host_session.drain_received().frames;
        if let Some(frame) = frames.into_iter().next() {
            received = Some(frame);
            true
        } else {
            false
        }
    });
    assert_eq!(received.expect("frame"), client_frame);
}

//
// ─── In-game control over a shared host-star socket ────────────────────────────
//

#[test]
fn control_flows_both_ways_after_host_star_game_starts() {
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
    let mut client = utils::lobby_client(ep1);

    host.poll().expect("seat the client");
    client.poll();
    let mut host = NetSession::start_host(host, None, &humans(2)).expect("start host");
    client.poll(); // receive the host's start signal
    let mut client = NetSession::start_client(client, &humans(2)).expect("start client");

    // Client → host: a pause request reaches the host over the shared socket.
    client
        .send_control(&ControlMessage::InGame(InGameMessage::PauseRequest {
            paused: true,
        }))
        .expect("client sends");
    host.drain_received(); // fills the control buffer from the shared socket
    assert_eq!(
        host.drain_control().messages,
        vec![(
            CLIENT,
            ControlMessage::InGame(InGameMessage::PauseRequest { paused: true })
        )],
    );

    // Host → client: the authoritative pause reaches the client.
    host.send_control(&ControlMessage::InGame(InGameMessage::PauseAt {
        proposer: Proposer::Player(HOST_PLAYER),
        tick: 50,
        paused: true,
    }))
    .expect("host sends");
    client.drain_received();
    assert_eq!(
        client.drain_control().messages,
        vec![(
            HOST_PEER,
            ControlMessage::InGame(InGameMessage::PauseAt {
                proposer: Proposer::Player(HOST_PLAYER),
                tick: 50,
                paused: true
            })
        )],
    );
}

/// Occupied human session slots `0..n`, matching a lobby the same size.
fn humans(n: usize) -> Vec<PlayerSlot> {
    (0..n)
        .map(|i| PlayerSlot::occupied(i as PlayerId, PlayerType::Human, None, None))
        .collect()
}
