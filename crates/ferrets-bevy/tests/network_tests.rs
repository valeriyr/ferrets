//! Two simulations driven over an in-process loopback transport stay in lockstep:
//! a command issued on one peer executes identically on both.

mod utils;

use bevy::prelude::*;
use ferrets_bevy::{
    DropConfig, NetworkPlugin, NetworkSession, SimulationPlugin, install_network_session,
};
use ferrets_math::FixedU64;
use ferrets_network::message::control::{ControlMessage, InGameMessage};
use ferrets_network::role::Role;
use ferrets_network::roster::Roster;
use ferrets_network::session::NetSession;
use ferrets_network::transport::NetworkTransport;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_pathfinder::{astar::Projection, nav_grid::NavGrid, nav_size::NavSize};
use ferrets_simulation::{
    checksum::state_checksum,
    command::PlayerCommand,
    components::location::Solidity,
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    map::Map,
    session::{
        FinishPolicy, GameResult, GameSession, ai_hosting::AiHosting, player_slot::PlayerSlot,
        player_type::PlayerType,
    },
    spawn,
};

use utils::GROUND;

/// Builds a networked app of `players` Human slots, whose local slot matches the
/// transport's peer. `players` is the roster a lobby would have agreed (slots
/// `0..players`), passed in rather than inferred from connectivity.
fn net_app(transport: LoopbackTransport, players: usize) -> App {
    let local = transport.local_peer() as u8;
    let roster = Roster::new((0..players as u64).collect());
    // Peer 0 coordinates the control plane, as the lobby would assign.
    let net = NetSession::over_shared(Box::new(transport), Role::Peer, roster, local == 0);
    assert_eq!(net.gameplay_ref().local_player(), local);

    let mut nav_grid = NavGrid::new(32, 32);
    nav_grid.add_layer(GROUND);
    let slots = (0..players)
        .map(|i| PlayerSlot::occupied(i as u8, PlayerType::Human, None))
        .collect();
    let session = GameSession::new(local, slots, AiHosting::Replicated, FinishPolicy::Endless);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        session,
        Map::new("test", Projection::Isometric, nav_grid, vec![]),
    ));
    app.add_plugins(NetworkPlugin);
    install_network_session(app.world_mut(), net);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None),
        );
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Advances both apps one fixed tick each, host first so its broadcast is visible
/// to the peer the same step.
fn step_both(host: &mut App, peer: &mut App, ticks: u32) {
    for _ in 0..ticks {
        host.world_mut().run_schedule(FixedUpdate);
        peer.world_mut().run_schedule(FixedUpdate);
    }
}

/// Advances every app one fixed tick each, in order, for `ticks` ticks.
fn step_all(apps: &mut [App], ticks: u32) {
    for _ in 0..ticks {
        for app in apps.iter_mut() {
            app.world_mut().run_schedule(FixedUpdate);
        }
    }
}

#[test]
fn spawn_command_on_one_peer_executes_on_both() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);

    // No fabricated AI frames here — frames must genuinely cross the transport.
    utils::push_command(
        &mut host,
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::pos(10, 10),
        },
    );

    step_both(&mut host, &mut peer, 6);

    // Both simulations ran the same command at the same tick.
    assert_eq!(utils::count_of_type(host.world_mut(), "soldier"), 1);
    assert_eq!(utils::count_of_type(peer.world_mut(), "soldier"), 1);
    assert_eq!(
        host.world().resource::<GameSession>().tick(),
        peer.world().resource::<GameSession>().tick(),
    );
}

#[test]
fn both_advance_in_lockstep_when_idle() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);

    step_both(&mut host, &mut peer, 10);

    let host_tick = host.world().resource::<GameSession>().tick();
    let peer_tick = peer.world().resource::<GameSession>().tick();
    assert_eq!(host_tick, peer_tick);
    assert!(host_tick > 0, "the tick should advance, not block");
}

#[test]
fn matched_peers_stay_in_sync_across_checksum_intervals() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);

    // Spawn on the host via the command pipeline, then run well past several
    // checksum intervals (interval = 8).
    utils::push_command(
        &mut host,
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::pos(10, 10),
        },
    );
    step_both(&mut host, &mut peer, 40);

    // Neither peer ever flagged a desync, and their state hashes agree.
    assert_eq!(host.world().resource::<GameSession>().result(), None);
    assert_eq!(peer.world().resource::<GameSession>().result(), None);
    assert_eq!(
        state_checksum(host.world()),
        state_checksum(peer.world()),
        "matched peers must hash identically",
    );
}

#[test]
fn three_peers_stay_in_lockstep_over_full_mesh() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();

    // Every peer issues its own spawn, so all three players contribute frames
    // that must cross the mesh and execute identically everywhere.
    for (i, app) in apps.iter_mut().enumerate() {
        utils::push_command(
            app,
            PlayerCommand::Spawn {
                type_name: "soldier".into(),
                position: utils::pos(10 + i as u32, 10),
            },
        );
    }

    step_all(&mut apps, 40);

    let checksums: Vec<u64> = apps
        .iter_mut()
        .map(|app| state_checksum(app.world()))
        .collect();
    assert!(
        checksums.windows(2).all(|w| w[0] == w[1]),
        "all three peers must hash identically, got {checksums:?}",
    );
    for app in &mut apps {
        assert_eq!(app.world().resource::<GameSession>().result(), None);
        assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 3);
    }
}

#[test]
fn frame_reaches_peer_with_no_direct_link_via_relay() {
    // Line topology 0—1—2: peers 0 and 2 are not directly linked, so 0's frames
    // reach 2 only if 1 relays them (and vice versa) — the roster is still the
    // full {0,1,2} a lobby would assign.
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (1, 2)])
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();

    // Peer 0 issues a spawn; it must cross 0 → 1 → 2 to execute on peer 2.
    utils::push_command(
        &mut apps[0],
        PlayerCommand::Spawn {
            type_name: "soldier".into(),
            position: utils::pos(10, 10),
        },
    );

    step_all(&mut apps, 12);

    // All three executed it (0 produced it, 1 got it directly, 2 via the relay)
    // and stay bit-identical — none blocked for want of a non-adjacent peer.
    for app in &mut apps {
        assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);
    }
    let checksum = state_checksum(apps[0].world());
    assert_eq!(checksum, state_checksum(apps[1].world()));
    assert_eq!(checksum, state_checksum(apps[2].world()));
}

#[test]
fn pause_takes_effect_at_same_tick_on_every_peer() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2); // peer 0
    let mut peer = net_app(b, 2); // peer 1 (peer 0 already coordinates control)

    // Reach lockstep, then the client requests a pause.
    step_both(&mut host, &mut peer, 6);
    assert!(!is_paused(&host) && !is_paused(&peer));
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: true }),
    );

    // Both freeze at the same tick, well before the step budget runs out.
    step_both(&mut host, &mut peer, 20);
    assert!(is_paused(&host) && is_paused(&peer), "both paused");
    let frozen = tick(&host);
    assert_eq!(frozen, tick(&peer), "frozen at the same tick");

    // The frozen tick does not advance, and the peers stay bit-identical.
    step_both(&mut host, &mut peer, 10);
    assert_eq!(tick(&host), frozen);
    assert_eq!(tick(&peer), frozen);
    assert_eq!(state_checksum(host.world()), state_checksum(peer.world()));

    // Resume: both leave the pause together and advance again.
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: false }),
    );
    step_both(&mut host, &mut peer, 10);
    assert!(!is_paused(&host) && !is_paused(&peer), "both resumed");
    assert_eq!(tick(&host), tick(&peer));
    assert!(tick(&host) > frozen, "advanced past the pause");
}

fn send_control(app: &mut App, message: ControlMessage) {
    app.world_mut()
        .get_non_send_resource_mut::<NetworkSession>()
        .expect("network session")
        .0
        .send_control(&message)
        .expect("send control");
}

fn is_paused(app: &App) -> bool {
    app.world().resource::<GameSession>().is_paused()
}

fn tick(app: &App) -> u32 {
    app.world().resource::<GameSession>().tick()
}

/// Pumps a chosen subset of apps (by index) one fixed tick each, for `ticks`
/// ticks — used to simulate some peers going silent.
fn step_some(apps: &mut [App], indices: &[usize], ticks: u32) {
    for _ in 0..ticks {
        for &i in indices {
            apps[i].world_mut().run_schedule(FixedUpdate);
        }
    }
}

#[test]
fn gone_peer_is_dropped_and_rest_continue_in_lockstep() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();
    for app in &mut apps {
        app.world_mut()
            .insert_resource(DropConfig { timeout_steps: 3 });
    }

    // Establish lockstep, then peer 2 goes silent (we stop pumping it).
    step_all(&mut apps, 6);
    step_some(&mut apps, &[0, 1], 40);

    // Peers 0 and 1 each dropped player 2 at the same tick and kept advancing
    // together — identical state, no premature game end.
    assert!(
        apps[0]
            .world()
            .resource::<GameSession>()
            .is_player_dropped(2)
    );
    assert!(
        apps[1]
            .world()
            .resource::<GameSession>()
            .is_player_dropped(2)
    );
    assert_eq!(apps[0].world().resource::<GameSession>().result(), None);
    assert_eq!(
        apps[0].world().resource::<GameSession>().tick(),
        apps[1].world().resource::<GameSession>().tick(),
    );
    assert_eq!(
        state_checksum(apps[0].world()),
        state_checksum(apps[1].world()),
        "the two survivors must stay bit-identical after the drop",
    );
}

#[test]
fn two_peer_disconnect_aborts_remaining_peer() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);
    host.world_mut()
        .insert_resource(DropConfig { timeout_steps: 3 });

    step_both(&mut host, &mut peer, 6);
    // The peer goes silent; pump only the host past the grace window. Missing its
    // only other player → it can't declare a winner → it aborts locally.
    for _ in 0..40 {
        host.world_mut().run_schedule(FixedUpdate);
    }

    assert_eq!(
        host.world().resource::<GameSession>().result(),
        Some(GameResult::Aborted),
    );
}

#[test]
fn briefly_silent_peer_is_not_dropped() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();
    for app in &mut apps {
        app.world_mut()
            .insert_resource(DropConfig { timeout_steps: 50 });
    }

    step_all(&mut apps, 6);
    // Peer 2 is silent for a while — long enough to block 0 and 1, but well under
    // the grace window — then resumes.
    step_some(&mut apps, &[0, 1], 10);
    step_all(&mut apps, 20);

    for app in &apps {
        assert!(!app.world().resource::<GameSession>().is_player_dropped(2));
        assert_eq!(app.world().resource::<GameSession>().result(), None);
    }
    assert_eq!(
        state_checksum(apps[0].world()),
        state_checksum(apps[2].world()),
        "a recovered peer stays in lockstep with the rest",
    );
}

#[test]
fn diverging_one_peer_trips_desync() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);

    step_both(&mut host, &mut peer, 5);

    // Force a divergence: spawn an entity on the host OUTSIDE the lockstep command
    // pipeline, so only the host's state (and checksum) changes.
    spawn::spawn_entity(host.world_mut(), "soldier", utils::pos(3, 3), Some(0))
        .expect("spawn perturbation");

    // Run past the next checksum interval plus delivery latency.
    step_both(&mut host, &mut peer, 20);

    assert!(
        matches!(
            host.world().resource::<GameSession>().result(),
            Some(GameResult::Desynchronization { .. })
        ),
        "the host should detect the divergence",
    );
}
