//! Two simulations driven over an in-process loopback transport stay in lockstep:
//! a command issued on one peer executes identically on both.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{
    DropConfig, NetworkPlugin, NetworkSession, ReplayPlugin, SimulationPlugin,
    install_network_session,
};
use ferrets_math::FixedU64;
use ferrets_network::message::control::{ControlMessage, InGameMessage};
use ferrets_network::role::Role;
use ferrets_network::roster::Roster;
use ferrets_network::session::NetSession;
use ferrets_network::transport::NetworkTransport;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_pathfinder::{astar::Projection, nav_grid::NavGrid, nav_size::NavSize};
use ferrets_replay::buffer::SharedBuffer;
use ferrets_replay::header::{RecordedGame, ReplayHeader};
use ferrets_replay::recorder::Recorder;
use ferrets_replay::replay::Replay;
use ferrets_simulation::{
    checksum::state_checksum,
    command::PlayerCommand,
    components::location::Solidity,
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    input::{InputFrames, PlayerFrame},
    map::Map,
    session::{
        GameResult, GameSession, Winner, ai_hosting::AiHosting, authority::Authority,
        drop_policy::DropPolicy, finish_policy::FinishPolicy, player_slot::PlayerSlot,
        player_type::PlayerType,
    },
    simulation_id::SimulationId,
    skirmish::Skirmish,
    spawn,
};

use utils::{GROUND, GROUND_LAYER};

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
    // that must cross the mesh and execute identically everywhere. The spawns
    // sit outside one another's acquisition range so the hostile soldiers
    // stand instead of thinning the count this test asserts.
    for (i, app) in apps.iter_mut().enumerate() {
        utils::push_command(
            app,
            PlayerCommand::Spawn {
                type_name: "soldier".into(),
                position: utils::pos(10 + 4 * i as u32, 10),
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
fn host_with_local_ai_drops_lone_silent_client() {
    // The silent client is the host's only remote player, but a locally
    // hosted AI keeps playing: the host is the drop authority and still has a
    // game to steer, so it drops the client instead of aborting.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), None]);
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
        PlayerSlot::occupied(2, PlayerType::Ai, None, None),
    ];
    let authority = Authority::Host {
        ai_hosting: AiHosting::Replicated,
    };
    let mut host = net_app_with_slots(a, roster.clone(), authority, slots.clone());
    let mut peer = net_app_with_slots(b, roster, authority, slots);
    host.world_mut()
        .insert_resource(DropConfig { timeout_steps: 3 });

    step_both(&mut host, &mut peer, 6);
    // The client goes silent; pump only the host past the grace window.
    for _ in 0..40 {
        host.world_mut().run_schedule(FixedUpdate);
    }

    let session = host.world().resource::<GameSession>();
    assert!(session.is_player_dropped(1));
    assert_eq!(session.result(), None);
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

#[test]
fn drop_stops_requiring_dropped_players_frames() {
    use bevy::ecs::system::RunSystemOnce;

    let (a, _b) = LoopbackTransport::pair();
    let mut app = net_app_with_roster(a, Roster::from_slots(vec![Some(0), Some(1), Some(99)]));
    let world = app.world_mut();
    // Player 1 delivered a real frame for tick 3 and died before tick 2's,
    // while players 0 and 2 stayed current; the session sits blocked at 2 with
    // the grace expired.
    for frame in [
        PlayerFrame {
            player: 1,
            tick: 3,
            commands: vec![PlayerCommand::Stop],
        },
        PlayerFrame::idle(0, 2),
        PlayerFrame::idle(2, 2),
    ] {
        world.resource_mut::<InputFrames>().push_frame(frame);
    }
    {
        let mut session = world.resource_mut::<GameSession>();
        session.advance_tick();
        session.advance_tick();
        session.set_blocked(true);
    }
    world.resource_mut::<DropConfig>().timeout_steps = 1;

    world
        .run_system_once(ferrets_bevy_plugin::detect_drops)
        .expect("run detect_drops");

    // The drop stops the tick from requiring player 1's input: the blocked
    // tick executes with the remaining players' frames alone.
    let session = world.resource::<GameSession>();
    assert!(session.is_player_dropped(1));
    assert!(!session.is_player_dropped(0));
    assert!(ferrets_simulation::game_loop::executor::tick(world, 2));
}

#[test]
fn survivors_converge_on_same_drop_tick_for_silent_player() {
    // Two live peers plus a third roster slot whose peer never speaks. Its
    // only frame reached peer A before the silence — peer B can only learn it
    // through A's frame-window relay, so both must execute it, block at the
    // same next tick, drop the silent player there, and keep playing in step.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = net_app_with_roster(a, roster.clone());
    let mut peer = net_app_with_roster(b, roster);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    host.world_mut()
        .resource_mut::<InputFrames>()
        .push_frame(PlayerFrame {
            player: 2,
            tick: 2,
            commands: vec![PlayerCommand::Stop],
        });

    step_both(&mut host, &mut peer, 60);

    // The relay carried the real tick-2 frame to B (tick 2 executed before
    // the drop, so both nodes required and recorded it)...
    let relayed = peer.world().resource::<InputFrames>().frames_in_range(2, 2);
    let frame = relayed.iter().find(|f| f.player == 2).expect("frame");
    assert_eq!(frame.commands, vec![PlayerCommand::Stop]);

    // ...and both survivors dropped the silent player and play on, in step.
    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(peer.world().resource::<GameSession>().is_player_dropped(2));
    let host_tick = host.world().resource::<GameSession>().tick();
    let peer_tick = peer.world().resource::<GameSession>().tick();
    assert_eq!(host_tick, peer_tick);
    assert!(host_tick > 3, "the survivors kept playing past the drop");
    assert_eq!(
        state_checksum(host.world_mut()),
        state_checksum(peer.world_mut()),
    );
}

#[test]
fn drop_for_already_executed_tick_ends_game_as_desync() {
    // A DropAt whose tick a node has already executed (with the player still
    // live) means the convergence drops rely on did not hold: the node's state
    // past that tick disagrees with the host's. It stops as a desync rather than
    // silently applying the contradictory drop.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();

    step_all(&mut apps, 6);
    let already_run = tick(&apps[1]) - 2;

    // The host (peer 0) is the only node a DropAt is trusted from.
    send_control(
        &mut apps[0],
        ControlMessage::InGame(InGameMessage::DropAt {
            player: 2,
            tick: already_run,
        }),
    );
    step_all(&mut apps, 4);

    assert_eq!(
        apps[1].world().resource::<GameSession>().result(),
        Some(GameResult::Desynchronization { tick: already_run }),
    );
    // The stale drop was refused, not applied.
    assert!(
        !apps[1]
            .world()
            .resource::<GameSession>()
            .is_player_dropped(2)
    );
}

#[test]
fn peer_authority_drops_silent_player_by_consensus() {
    // Same silent-phantom scenario as the host-authority convergence test, but
    // no node is special: each survivor casts its stall observation on the
    // control mesh and the drop commits only on unanimity.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = net_app_configured(b, roster, Authority::Peers);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;

    step_both(&mut host, &mut peer, 60);

    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(peer.world().resource::<GameSession>().is_player_dropped(2));
    align_ticks(&mut host, &mut peer);
    let tick = host.world().resource::<GameSession>().tick();
    assert!(tick > 3, "the survivors kept playing past the drop");
    assert_eq!(
        state_checksum(host.world_mut()),
        state_checksum(peer.world_mut()),
    );
}

#[test]
fn peer_consensus_drops_silent_player_despite_environment_slot() {
    // An environment slot's frames are computed on every node, so it can
    // neither stall the tick nor cast a consensus vote — the survivors'
    // unanimity must not wait for one.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99), None]);
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
        PlayerSlot::occupied(2, PlayerType::Human, None, None),
        PlayerSlot::environment(3),
    ];
    let mut host = net_app_with_slots(a, roster.clone(), Authority::Peers, slots.clone());
    let mut peer = net_app_with_slots(b, roster, Authority::Peers, slots);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;

    step_both(&mut host, &mut peer, 60);

    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(peer.world().resource::<GameSession>().is_player_dropped(2));
    assert!(
        !host.world().resource::<GameSession>().is_player_dropped(3),
        "the environment slot is not a stall"
    );
    align_ticks(&mut host, &mut peer);
    assert_eq!(
        state_checksum(host.world_mut()),
        state_checksum(peer.world_mut()),
    );
}

#[test]
fn consensus_votes_cross_broken_control_link_via_flooding() {
    // Line topology 0 - 1 - 2: peers 0 and 2 share no direct link, so their
    // stall votes about the silent fourth slot can only meet through peer 1
    // forwarding what it learns. All three must still commit the same drop.
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(2), Some(99)]);
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (1, 2)])
        .into_iter()
        .map(|t| net_app_configured(t, roster.clone(), Authority::Peers))
        .collect();
    for app in &mut apps {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
    }

    step_all(&mut apps, 50);

    for app in &apps {
        assert!(app.world().resource::<GameSession>().is_player_dropped(3));
        assert_eq!(app.world().resource::<GameSession>().result(), None);
    }
    let (left, right) = apps.split_at_mut(1);
    align_ticks(&mut left[0], &mut right[0]);
    assert_eq!(
        state_checksum(left[0].world_mut()),
        state_checksum(right[0].world_mut()),
    );
}

#[test]
fn losing_control_link_to_host_aborts_client() {
    // Under host authority a client whose control link to the host died can
    // no longer be steered: no DropAt or PauseAt will ever arrive, however
    // healthy the gameplay traffic looks.
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app(a, 2);
    let mut peer = net_app(b, 2);
    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::ControlLinks>()
        .lost
        .insert(0);

    step_both(&mut host, &mut peer, 3);

    assert_eq!(
        peer.world().resource::<GameSession>().result(),
        Some(GameResult::Aborted),
    );
    assert_eq!(host.world().resource::<GameSession>().result(), None);
}

#[test]
fn losing_every_control_link_aborts_decentralized_node() {
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1)]);
    let mut host = net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = net_app_configured(b, roster, Authority::Peers);
    host.world_mut()
        .resource_mut::<ferrets_bevy_plugin::ControlLinks>()
        .lost
        .insert(1);

    step_both(&mut host, &mut peer, 3);

    assert_eq!(
        host.world().resource::<GameSession>().result(),
        Some(GameResult::Aborted),
    );
}

#[test]
fn losing_control_link_to_eliminated_player_does_not_abort() {
    // The same lost link as above, but the player behind it was eliminated
    // first: its node is expected to be gone, so the dead link proves no
    // partition and the survivor plays on.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1)]);
    let mut host = net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = net_app_configured(b, roster, Authority::Peers);
    host.world_mut()
        .resource_mut::<GameSession>()
        .eliminate_player(1, 1);
    step_both(&mut host, &mut peer, 2);
    host.world_mut()
        .resource_mut::<ferrets_bevy_plugin::ControlLinks>()
        .lost
        .insert(1);

    step_both(&mut host, &mut peer, 3);

    assert_eq!(host.world().resource::<GameSession>().result(), None);
}

#[test]
fn host_death_aborts_survivors_under_host_authority() {
    // The host itself goes silent. Under host authority no DropAt can ever
    // arrive for it, so the survivors end their sessions rather than playing a
    // game nobody can steer.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();
    for app in &mut apps {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 3;
    }

    step_all(&mut apps, 6);
    step_some(&mut apps, &[1, 2], 40);

    for survivor in [1, 2] {
        assert_eq!(
            apps[survivor].world().resource::<GameSession>().result(),
            Some(GameResult::Aborted),
        );
        assert!(
            !apps[survivor]
                .world()
                .resource::<GameSession>()
                .is_player_dropped(0),
            "the host is not droppable without an authority",
        );
    }
}

#[test]
fn host_death_is_survived_under_peer_authority() {
    // The identical scenario under peer authority: the lobby host is nobody
    // special once the game runs, so the survivors drop it by consensus and
    // play on in lockstep.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .zip([Authority::Peers; 3])
        .map(|(t, authority)| net_app_configured(t, Roster::new((0..3).collect()), authority))
        .collect();
    for app in &mut apps {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 3;
    }

    step_all(&mut apps, 6);
    step_some(&mut apps, &[1, 2], 40);

    for survivor in [1, 2] {
        let session = apps[survivor].world().resource::<GameSession>();
        assert!(session.is_player_dropped(0));
        assert_eq!(session.result(), None);
    }
    let (left, right) = apps.split_at_mut(2);
    align_ticks(&mut left[1], &mut right[0]);
    assert_eq!(
        state_checksum(left[1].world_mut()),
        state_checksum(right[0].world_mut()),
    );
}

#[test]
fn manual_policy_holds_drop_until_game_approves() {
    // The deciding host runs the manual policy: the stall is surfaced but
    // nobody is dropped past the grace window until the game (a wait dialog,
    // some day) approves the player.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = net_app_with_roster(a, roster.clone());
    let mut peer = net_app_with_roster(b, roster);
    for app in [&mut host, &mut peer] {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_drop_policy(DropPolicy::Manual);
    }

    step_both(&mut host, &mut peer, 40);

    // Well past the grace window: still stalled, still nobody dropped.
    assert!(!host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(!peer.world().resource::<GameSession>().is_player_dropped(2));
    let stall = host.world().resource::<ferrets_bevy_plugin::Stall>();
    assert_eq!(
        stall.0.as_ref().map(|info| info.missing.clone()),
        Some(vec![2]),
    );

    // The game approves the drop on the deciding node; both nodes apply it.
    host.world_mut()
        .resource_mut::<ferrets_bevy_plugin::DropIntent>()
        .0
        .push(2);
    step_both(&mut host, &mut peer, 10);

    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(peer.world().resource::<GameSession>().is_player_dropped(2));
    assert_eq!(
        host.world().resource::<GameSession>().tick(),
        peer.world().resource::<GameSession>().tick(),
    );
}

#[test]
fn peer_authority_pause_freezes_and_resumes_both_at_same_tick() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = net_app_configured(a, Roster::new((0..2).collect()), Authority::Peers);
    let mut peer = net_app_configured(b, Roster::new((0..2).collect()), Authority::Peers);
    step_both(&mut host, &mut peer, 4);

    // The NON-host proposes the pause: there is no authority to ask.
    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(true);
    step_both(&mut host, &mut peer, 12);

    assert!(host.world().resource::<GameSession>().is_paused());
    assert!(peer.world().resource::<GameSession>().is_paused());
    let frozen = host.world().resource::<GameSession>().tick();
    assert_eq!(frozen, peer.world().resource::<GameSession>().tick());

    step_both(&mut host, &mut peer, 5);
    assert_eq!(host.world().resource::<GameSession>().tick(), frozen);

    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(false);
    step_both(&mut host, &mut peer, 12);

    assert!(!host.world().resource::<GameSession>().is_paused());
    assert!(!peer.world().resource::<GameSession>().is_paused());
    assert!(host.world().resource::<GameSession>().tick() > frozen);
    align_ticks(&mut host, &mut peer);
    assert_eq!(
        state_checksum(host.world_mut()),
        state_checksum(peer.world_mut()),
    );
}

#[test]
fn stale_pause_proposal_does_not_resurrect_after_its_tick_passed() {
    // Pause and resume normally, then replay a copy of the original pause —
    // a flooded duplicate arriving long after the change was applied and
    // discarded. It must be ignored, not re-applied.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::new((0..2).collect());
    let mut host = net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = net_app_configured(b, roster, Authority::Peers);
    step_both(&mut host, &mut peer, 4);

    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(true);
    step_both(&mut host, &mut peer, 12);
    let frozen = tick(&host);
    assert!(host.world().resource::<GameSession>().is_paused());

    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(false);
    step_both(&mut host, &mut peer, 12);
    assert!(!peer.world().resource::<GameSession>().is_paused());

    // The copy names the SENDER as proposer, so the receiver cannot dismiss
    // it as its own echo — only the stale tick identifies it as dead.
    send_control(
        &mut host,
        ControlMessage::InGame(InGameMessage::PauseAt {
            proposer: 0,
            tick: frozen,
            paused: true,
        }),
    );
    step_both(&mut host, &mut peer, 4);

    assert!(!host.world().resource::<GameSession>().is_paused());
    assert!(!peer.world().resource::<GameSession>().is_paused());
}

#[test]
fn game_with_mid_game_drop_records_and_replays_identically() {
    // Same silent-phantom scenario as above, but the phantom's lone frame is
    // state-changing and the host records the game. The recording must keep
    // the frame (its tick executed while the player was live) yet not carry
    // anything for the player past the drop tick — playback re-verifies every
    // recorded checksum, so either mistake fails the assertions below.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = net_app_with_roster(a, roster.clone());
    let mut peer = net_app_with_roster(b, roster);
    host.add_plugins(ReplayPlugin);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;

    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: three_human_slots(),
        map: "test".to_string(),
        finish_policy: FinishPolicy::Endless,
    }));
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    ferrets_bevy_plugin::install_replay_recorder(host.world_mut(), recorder);

    host.world_mut()
        .resource_mut::<InputFrames>()
        .push_frame(PlayerFrame {
            player: 2,
            tick: 2,
            commands: vec![PlayerCommand::Spawn {
                type_name: "soldier".into(),
                position: utils::pos(20, 20),
            }],
        });

    for _ in 0..60 {
        host.world_mut().run_schedule(FixedUpdate);
        // The recorder runs after the tick, in `FixedLast` (only the recording
        // app has that schedule).
        host.world_mut().run_schedule(FixedLast);
        peer.world_mut().run_schedule(FixedUpdate);
    }
    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert_eq!(utils::count_of_type(host.world_mut(), "soldier"), 1);

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = utils::make_app(three_human_slots());
    playback.add_plugins(ReplayPlugin);
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(harness_soldier());
        registry.validate();
    }
    ferrets_bevy_plugin::install_replay_playback(playback.world_mut(), replay);
    playback.world_mut().resource_mut::<GameSession>().start();

    for _ in 0..70 {
        playback.world_mut().run_schedule(FixedUpdate);
        playback.world_mut().run_schedule(FixedLast);
    }

    let watched = playback
        .world()
        .resource::<ferrets_bevy_plugin::ReplayPlayback>();
    assert!(
        watched.is_done(),
        "playback should reach the recording's end"
    );
    assert_eq!(watched.mismatch(), None);
    assert_eq!(utils::count_of_type(playback.world_mut(), "soldier"), 1);
    // Playback re-applied the recorded drop, so the player is dropped here too —
    // replayed as dropped, not as idle-but-present.
    assert!(
        playback
            .world()
            .resource::<GameSession>()
            .is_player_dropped(2)
    );
}

#[test]
fn drop_decided_victory_replays_to_same_result() {
    // Allied players 0 and 1 against the phantom player 2; everyone starts with
    // a base, then the phantom goes silent and is dropped. Under LastStanding
    // that drop is what ends the game — the phantom's lingering base is excluded
    // from the survivors, leaving one side — so the recording must carry the
    // drop for playback to reach the same Victory. Without it, playback would
    // see two opposing sides and never end.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let authority = Authority::Host {
        ai_hosting: AiHosting::Replicated,
    };
    let mut host = net_app_with_slots(a, roster.clone(), authority, teamed_human_slots());
    let mut peer = net_app_with_slots(b, roster, authority, teamed_human_slots());
    host.add_plugins(ReplayPlugin);
    // Starting units, like a game's setup would place them — spawned identically
    // on every node (not recorded as input, so playback re-creates them the same
    // way).
    for app in [&mut host, &mut peer] {
        spawn_starting_units(app);
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding);
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
    }

    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: teamed_human_slots(),
        map: "test".to_string(),
        finish_policy: FinishPolicy::LastStanding,
    }));
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    ferrets_bevy_plugin::install_replay_recorder(host.world_mut(), recorder);

    // The phantom sends nothing, so the host blocks, waits out the grace window,
    // and drops it — ending the game as a win for the allied team.
    for _ in 0..60 {
        if host.world().resource::<GameSession>().result().is_some() {
            break;
        }
        host.world_mut().run_schedule(FixedUpdate);
        host.world_mut().run_schedule(FixedLast);
        peer.world_mut().run_schedule(FixedUpdate);
    }
    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert_eq!(
        host.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Team(1)
        }),
    );

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = utils::make_app(teamed_human_slots());
    playback.add_plugins(ReplayPlugin);
    playback
        .world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    spawn_starting_units(&mut playback);
    ferrets_bevy_plugin::install_replay_playback(playback.world_mut(), replay);
    playback.world_mut().resource_mut::<GameSession>().start();

    for _ in 0..70 {
        if playback
            .world()
            .resource::<GameSession>()
            .result()
            .is_some()
        {
            break;
        }
        playback.world_mut().run_schedule(FixedUpdate);
        playback.world_mut().run_schedule(FixedLast);
    }

    assert_eq!(
        playback
            .world()
            .resource::<ferrets_bevy_plugin::ReplayPlayback>()
            .mismatch(),
        None,
    );
    assert_eq!(
        playback.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Team(1)
        }),
        "the replay must reach the same drop-decided victory",
    );
}

#[test]
fn lone_winner_victory_past_drop_replays_to_same_result() {
    // A teamless free-for-all whose phantom third slot is dropped mid-game,
    // then player 1 is eliminated in combat: the drop leaves two opposing
    // survivors, so it is the elimination that ends the game — with a lone
    // teamless winner. The recording carries the drop and playback re-derives
    // the elimination, so it must reach the same Victory for the same lone
    // player.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = net_app_with_roster(a, roster.clone());
    let mut peer = net_app_with_roster(b, roster);
    host.add_plugins(ReplayPlugin);
    let mut ids = Vec::new();
    for app in [&mut host, &mut peer] {
        ids.push(spawn_lone_winner_lineup(app));
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding);
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
    }
    let (attacker, target) = ids[0];

    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: three_human_slots(),
        map: "test".to_string(),
        finish_policy: FinishPolicy::LastStanding,
    }));
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    ferrets_bevy_plugin::install_replay_recorder(host.world_mut(), recorder);

    // Player 0 sends its soldier onto player 1's base while the phantom stays
    // silent: the short grace lands the drop first, the kill ends the game.
    utils::select(&mut host, attacker);
    utils::push_command(
        &mut host,
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
    for _ in 0..80 {
        if host.world().resource::<GameSession>().result().is_some() {
            break;
        }
        host.world_mut().run_schedule(FixedUpdate);
        host.world_mut().run_schedule(FixedLast);
        peer.world_mut().run_schedule(FixedUpdate);
    }
    let session = host.world().resource::<GameSession>();
    assert!(session.is_player_dropped(2));
    assert_eq!(
        session.result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
    );

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = utils::make_app(three_human_slots());
    playback.add_plugins(ReplayPlugin);
    playback
        .world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    spawn_lone_winner_lineup(&mut playback);
    ferrets_bevy_plugin::install_replay_playback(playback.world_mut(), replay);
    playback.world_mut().resource_mut::<GameSession>().start();

    for _ in 0..90 {
        if playback
            .world()
            .resource::<GameSession>()
            .result()
            .is_some()
        {
            break;
        }
        playback.world_mut().run_schedule(FixedUpdate);
        playback.world_mut().run_schedule(FixedLast);
    }

    assert_eq!(
        playback
            .world()
            .resource::<ferrets_bevy_plugin::ReplayPlayback>()
            .mismatch(),
        None,
    );
    assert_eq!(
        playback.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
        "the replay must reach the same lone-winner victory",
    );
}

#[test]
fn non_drop_victory_replays_to_same_result() {
    // A plain last-standing win with no drop involved: player 0's soldier
    // destroys player 1's base, ending the game by elimination. The tick whose
    // kill ends the game is the final one recorded, so this exercises the same
    // final-tick recording as the drop case for an ordinary outcome — the replay
    // must reach the same Victory.
    let mut record_app = combat_victory_app();
    record_app.add_plugins(ReplayPlugin);
    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: two_human_slots(),
        map: "test".to_string(),
        finish_policy: FinishPolicy::LastStanding,
    }));
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    ferrets_bevy_plugin::install_replay_recorder(record_app.world_mut(), recorder);

    let (attacker, enemy) = spawn_combatants(&mut record_app);
    utils::select(&mut record_app, attacker);
    utils::push_command(
        &mut record_app,
        PlayerCommand::SendToEntity {
            target: enemy,
            flush: true,
        },
    );
    step_local_recording(&mut record_app, 80);
    assert_eq!(
        record_app.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
    );

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = combat_victory_app();
    playback.add_plugins(ReplayPlugin);
    spawn_combatants(&mut playback);
    ferrets_bevy_plugin::install_replay_playback(playback.world_mut(), replay);

    for _ in 0..90 {
        if playback
            .world()
            .resource::<GameSession>()
            .result()
            .is_some()
        {
            break;
        }
        playback.world_mut().run_schedule(FixedUpdate);
        playback.world_mut().run_schedule(FixedLast);
    }

    assert_eq!(
        playback
            .world()
            .resource::<ferrets_bevy_plugin::ReplayPlayback>()
            .mismatch(),
        None,
    );
    assert_eq!(
        playback.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
        "a non-drop last-standing win must replay to the same result",
    );
}

#[test]
fn eliminated_player_node_freezing_does_not_stall_survivors() {
    // A three-way free-for-all under the manual drop policy — the policy that
    // would hold a stalled tick forever. Player 0 destroys player 2's last
    // building: node 2 finishes with Defeat and stops feeding frames, but every
    // survivor derives the same elimination from its own simulation and stops
    // requiring player 2's input — the game sails on in lockstep with no stall
    // and nobody dropped.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| net_app(t, 3))
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        ids.push(spawn_ffa_combatants(app));
        let mut session = app.world_mut().resource_mut::<GameSession>();
        session.set_finish_policy(FinishPolicy::LastStanding);
        session.set_drop_policy(DropPolicy::Manual);
    }
    let (attacker, target) = ids[0];

    utils::select(&mut apps[0], attacker);
    utils::push_command(
        &mut apps[0],
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
    step_all(&mut apps, 80);

    // Node 2 learned of its own defeat and froze there...
    assert_eq!(
        apps[2].world().resource::<GameSession>().result(),
        Some(GameResult::Defeat),
    );
    // ...while the survivors played past it, eliminated-not-dropped.
    for survivor in [0, 1] {
        let session = apps[survivor].world().resource::<GameSession>();
        assert_eq!(session.result(), None);
        assert!(session.is_player_eliminated(2));
        assert!(!session.is_player_dropped(2));
    }
    assert!(
        tick(&apps[0]) > tick(&apps[2]) + 30,
        "the survivors kept ticking after node 2 froze",
    );
    let (left, right) = apps.split_at_mut(1);
    align_ticks(&mut left[0], &mut right[0]);
    assert_eq!(
        state_checksum(left[0].world()),
        state_checksum(right[0].world()),
        "the two survivors must stay bit-identical past the elimination",
    );
}

#[test]
fn game_with_mid_game_elimination_records_and_replays_identically() {
    // A free-for-all where player 2 is eliminated mid-game and the match goes
    // on. The recording carries nothing for player 2 from the elimination tick
    // on — yet no drop, since none happened — so playback must re-derive the
    // elimination from the simulation itself to stop requiring (and supplying)
    // the missing frames. Checksum verification catches any divergence.
    let mut record_app = ffa_elimination_app();
    record_app.add_plugins(ReplayPlugin);
    let buffer = SharedBuffer::default();
    let header = ReplayHeader::new(RecordedGame::Skirmish(Skirmish {
        slots: three_human_slots(),
        map: "test".to_string(),
        finish_policy: FinishPolicy::LastStanding,
    }));
    let recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
    ferrets_bevy_plugin::install_replay_recorder(record_app.world_mut(), recorder);

    let (attacker, target) = spawn_ffa_combatants(&mut record_app);
    utils::select(&mut record_app, attacker);
    utils::push_command(
        &mut record_app,
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
    step_local_recording(&mut record_app, 60);

    // The elimination happened mid-recording and the game continued past it.
    let session = record_app.world().resource::<GameSession>();
    assert!(session.is_player_eliminated(2));
    assert_eq!(session.result(), None);

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = ffa_elimination_app();
    playback.add_plugins(ReplayPlugin);
    spawn_ffa_combatants(&mut playback);
    ferrets_bevy_plugin::install_replay_playback(playback.world_mut(), replay);

    for _ in 0..80 {
        if playback
            .world()
            .resource::<ferrets_bevy_plugin::ReplayPlayback>()
            .is_done()
        {
            break;
        }
        playback.world_mut().run_schedule(FixedUpdate);
        playback.world_mut().run_schedule(FixedLast);
    }

    let watched = playback
        .world()
        .resource::<ferrets_bevy_plugin::ReplayPlayback>();
    assert!(
        watched.is_done(),
        "playback should reach the recording's end"
    );
    assert_eq!(watched.mismatch(), None);
    let session = playback.world().resource::<GameSession>();
    assert!(session.is_player_eliminated(2));
    assert!(!session.is_player_dropped(2));
    assert_eq!(session.result(), None);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

/// Builds a networked app of `players` Human slots, whose local slot matches the
/// transport's peer. `players` is the roster a lobby would have agreed (slots
/// `0..players`), passed in rather than inferred from connectivity.
fn net_app(transport: LoopbackTransport, players: usize) -> App {
    net_app_with_roster(transport, Roster::new((0..players as u64).collect()))
}

/// Like [`net_app`], with an explicit roster (e.g. a slot whose peer will
/// never speak).
fn net_app_with_roster(transport: LoopbackTransport, roster: Roster) -> App {
    net_app_configured(
        transport,
        roster,
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
    )
}

/// Like [`net_app_with_roster`], with an explicit decision authority.
fn net_app_configured(transport: LoopbackTransport, roster: Roster, authority: Authority) -> App {
    let slots = (0..roster.len())
        .map(|i| PlayerSlot::occupied(i as u8, PlayerType::Human, None, None))
        .collect();
    net_app_with_slots(transport, roster, authority, slots)
}

/// Like [`net_app_configured`], with explicit session slots (e.g. allied ones).
fn net_app_with_slots(
    transport: LoopbackTransport,
    roster: Roster,
    authority: Authority,
    slots: Vec<PlayerSlot>,
) -> App {
    let local = roster
        .player_of(transport.local_peer())
        .expect("local peer is in the roster");
    // Peer 0 is the host node, as the lobby would assign.
    let net = NetSession::over_shared(Box::new(transport), Role::Peer, roster);
    assert_eq!(net.gameplay_ref().local_player(), local);

    let mut nav_grid = NavGrid::new(32, 32);
    nav_grid.add_layer(GROUND);
    let session = GameSession::configured(
        local,
        slots,
        "test",
        authority,
        DropPolicy::Automatic,
        FinishPolicy::Endless,
    );

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        session,
        Map::new("test", Projection::Isometric, nav_grid, vec![]),
    ));
    app.add_plugins(NetworkPlugin);
    // Supplies idle frames for AI slots with no installed runtime, as in a
    // real game; a no-op for the all-human rosters.
    app.add_plugins(ferrets_bevy_plugin::ai::AiPlugin);
    install_network_session(app.world_mut(), net);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// The one mobile entity type the harness games use. Armed, so a game can
/// destroy a building through the command pipeline; nothing attacks unordered.
fn harness_soldier() -> EntityTypeDef {
    EntityTypeDef::new("soldier")
        .with_location(GROUND, NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_num(0.5))
        .with_health(30)
        .with_dying(2, None)
        .with_attack(10, 1, 1, 2, 2)
}

/// A standing building — the presence the `LastStanding` rule counts. Immobile,
/// destructible, no combat of its own.
fn harness_base() -> EntityTypeDef {
    EntityTypeDef::new("base")
        .with_location(GROUND, NavSize::ONE, Solidity::Solid)
        .with_health(30)
        .with_dying(2, None)
        .with_tags(["building"])
}

/// Three occupied human slots — the harness roster as session slots.
fn three_human_slots() -> Vec<PlayerSlot> {
    (0..3)
        .map(|i| PlayerSlot::occupied(i, PlayerType::Human, None, None))
        .collect()
}

/// Three occupied human slots with players 0 and 1 allied against the teamless
/// slot 2 — so excluding slot 2 leaves a single side standing.
fn teamed_human_slots() -> Vec<PlayerSlot> {
    vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(2, PlayerType::Human, None, None),
    ]
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

/// Steps whichever app lags until both report the same tick. Lockstep permits
/// a bounded skew (decisions commit on different steps per node), so state
/// comparisons first need a common tick.
fn align_ticks(a: &mut App, b: &mut App) {
    for _ in 0..8 {
        let tick_a = a.world().resource::<GameSession>().tick();
        let tick_b = b.world().resource::<GameSession>().tick();
        match tick_a.cmp(&tick_b) {
            std::cmp::Ordering::Less => a.world_mut().run_schedule(FixedUpdate),
            std::cmp::Ordering::Greater => b.world_mut().run_schedule(FixedUpdate),
            std::cmp::Ordering::Equal => return,
        }
    }
    panic!("ticks failed to align");
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

/// Places every player's starting base, in a fixed order so their
/// [`SimulationId`]s — and thus the state checksum — match across every node and
/// the replay. A base is the presence the win rule counts.
fn spawn_starting_units(app: &mut App) {
    let world = app.world_mut();
    spawn::spawn_entity(world, "base", utils::pos(5, 5), Some(0)).expect("player 0 base");
    spawn::spawn_entity(world, "base", utils::pos(10, 10), Some(1)).expect("player 1 base");
    spawn::spawn_entity(world, "base", utils::pos(20, 20), Some(2)).expect("phantom base");
}

fn two_human_slots() -> Vec<PlayerSlot> {
    (0..2)
        .map(|i| PlayerSlot::occupied(i, PlayerType::Human, None, None))
        .collect()
}

/// A fresh two-player app with a combat-capable soldier, `LastStanding`, and the
/// session started — the setup a last-standing win is recorded and replayed on.
fn combat_victory_app() -> App {
    let mut app = utils::make_app(two_human_slots());
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    let mut session = app.world_mut().resource_mut::<GameSession>();
    session.set_finish_policy(FinishPolicy::LastStanding);
    session.start();
    app
}

/// A fresh three-player free-for-all app with the armed soldier and the base,
/// `LastStanding`, and the session started — the setup a mid-game elimination
/// is recorded and replayed on.
fn ffa_elimination_app() -> App {
    let mut app = utils::make_app(three_human_slots());
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    let mut session = app.world_mut().resource_mut::<GameSession>();
    session.set_finish_policy(FinishPolicy::LastStanding);
    session.start();
    app
}

/// Sets up a three-way free-for-all: a base per player, plus player 0's soldier
/// next to player 2's base. Spawned in a fixed order so ids — and thus the state
/// checksum — match across every node and the replay. Returns the attacker and
/// player 2's base [`SimulationId`]s; destroying that base eliminates player 2
/// while players 0 and 1, unallied, fight on.
fn spawn_ffa_combatants(app: &mut App) -> (SimulationId, SimulationId) {
    let world = app.world_mut();
    spawn::spawn_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    spawn::spawn_entity(world, "base", utils::pos(25, 25), Some(1)).expect("player 1 base");
    let (_, target) =
        spawn::spawn_entity(world, "base", utils::pos(6, 5), Some(2)).expect("player 2 base");
    let (_, attacker) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
    (attacker, target)
}

/// Sets up the lone-winner lineup: a base per slot — the phantom slot 2
/// included — plus player 0's soldier next to player 1's base. Spawned in a
/// fixed order so ids — and thus the state checksum — match across every node
/// and the replay. Returns the attacker and player 1's base [`SimulationId`]s.
fn spawn_lone_winner_lineup(app: &mut App) -> (SimulationId, SimulationId) {
    let world = app.world_mut();
    spawn::spawn_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    let (_, target) =
        spawn::spawn_entity(world, "base", utils::pos(6, 5), Some(1)).expect("player 1 base");
    spawn::spawn_entity(world, "base", utils::pos(25, 25), Some(2)).expect("phantom base");
    let (_, attacker) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
    (attacker, target)
}

/// Sets up a last-standing combat: player 0 keeps a base and an attacking
/// soldier next to player 1's base, the target. Spawned in a fixed order so ids
/// match across record and replay. Returns the attacker and the target base
/// [`SimulationId`]s. Destroying the target leaves player 0 the last one with a
/// building standing.
fn spawn_combatants(app: &mut App) -> (SimulationId, SimulationId) {
    let world = app.world_mut();
    // Player 0's own base keeps it in the game after the kill.
    spawn::spawn_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    let (_, attacker) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
    let (_, enemy_base) =
        spawn::spawn_entity(world, "base", utils::pos(6, 5), Some(1)).expect("enemy base");
    (attacker, enemy_base)
}

/// Advances a local recording app up to `ticks` ticks (stopping once the game
/// ends), feeding an idle frame for every non-local slot each tick — a local
/// game has no network source for them — and running `FixedLast` so the recorder
/// captures each completed tick.
fn step_local_recording(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        if app.world().resource::<GameSession>().result().is_some() {
            break;
        }
        let (tick, others) = {
            let session = app.world().resource::<GameSession>();
            let local = session.local_player();
            let others: Vec<_> = session
                .slots()
                .iter()
                .map(|slot| slot.id())
                .filter(|&id| id != local)
                .collect();
            (session.tick(), others)
        };
        for player in others {
            app.world_mut()
                .resource_mut::<InputFrames>()
                .push_frame(PlayerFrame::idle(player, tick));
        }
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().run_schedule(FixedLast);
    }
}
