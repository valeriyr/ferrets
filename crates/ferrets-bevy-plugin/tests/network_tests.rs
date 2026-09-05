//! Two simulations driven over an in-process loopback transport stay in lockstep:
//! a command issued on one peer executes identically on both.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{DropConfig, FrameMargins, NetworkSession, PeerCapacities, ReplayPlugin};
use ferrets_content::registry::ContentRegistry;
use ferrets_math::FixedU64;
use ferrets_network::{
    message::control::{ControlMessage, InGameMessage, Proposer},
    roster::Roster,
    transport::loopback::LoopbackTransport,
};
use ferrets_replay::replay::Replay;
use ferrets_simulation::{
    checksum,
    command::PlayerCommand,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::{
        GameResult, GameSession, Winner, ai_hosting::AiHosting, ai_vision::AiVision,
        authority::Authority, defeat_conduct::DefeatConduct, drop_policy::DropPolicy,
        elimination_scope::EliminationScope, finish_policy::FinishPolicy, game_speed::GameSpeed,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
};

use utils::{GROUND, GROUND_LAYER, NOMINAL_MILLIS};

#[test]
fn spawn_command_on_one_peer_executes_on_both() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);

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
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);

    step_both(&mut host, &mut peer, 10);

    let host_tick = host.world().resource::<GameSession>().tick();
    let peer_tick = peer.world().resource::<GameSession>().tick();
    assert_eq!(host_tick, peer_tick);
    assert!(host_tick > 0, "the tick should advance, not block");
}

#[test]
fn matched_peers_stay_in_sync_across_checksum_intervals() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);

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
        checksum::state_checksum(host.world()),
        checksum::state_checksum(peer.world()),
        "matched peers must hash identically",
    );
}

#[test]
fn three_peers_stay_in_lockstep_over_full_mesh() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| utils::net_app(t, 3))
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
        .map(|app| checksum::state_checksum(app.world()))
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
        .map(|t| utils::net_app(t, 3))
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
    let checksum = checksum::state_checksum(apps[0].world());
    assert_eq!(checksum, checksum::state_checksum(apps[1].world()));
    assert_eq!(checksum, checksum::state_checksum(apps[2].world()));
}

#[test]
fn pause_takes_effect_at_same_tick_on_every_peer() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2); // peer 0
    let mut peer = utils::net_app(b, 2); // peer 1 (peer 0 already coordinates control)

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
    let frozen = utils::tick(&host);
    assert_eq!(frozen, utils::tick(&peer), "frozen at the same tick");

    // The frozen tick does not advance, and the peers stay bit-identical.
    step_both(&mut host, &mut peer, 10);
    assert_eq!(utils::tick(&host), frozen);
    assert_eq!(utils::tick(&peer), frozen);
    assert_eq!(
        checksum::state_checksum(host.world()),
        checksum::state_checksum(peer.world())
    );

    // Resume: both leave the pause together and advance again.
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: false }),
    );
    step_both(&mut host, &mut peer, 10);
    assert!(!is_paused(&host) && !is_paused(&peer), "both resumed");
    assert_eq!(utils::tick(&host), utils::tick(&peer));
    assert!(utils::tick(&host) > frozen, "advanced past the pause");
}

#[test]
fn speed_takes_effect_at_same_tick_on_every_peer() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2); // peer 0
    let mut peer = utils::net_app(b, 2); // peer 1 (peer 0 already coordinates control)

    // Reach lockstep, then the client asks to run at double speed.
    step_both(&mut host, &mut peer, 6);
    assert_eq!(speed(&host), GameSpeed::NORMAL);
    assert_eq!(speed(&peer), GameSpeed::NORMAL);
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::SpeedRequest {
            speed: double_speed(),
        }),
    );

    // Both adopt it, and both were on the same tick when they did: the speed a
    // node runs at is a function of the tick it is on, so a tick they share must
    // carry the same speed.
    step_both(&mut host, &mut peer, 20);
    assert_eq!(speed(&host), double_speed(), "host at double speed");
    assert_eq!(speed(&peer), double_speed(), "peer at double speed");
    assert_eq!(utils::tick(&host), utils::tick(&peer), "still in lockstep");
    assert_eq!(
        checksum::state_checksum(host.world()),
        checksum::state_checksum(peer.world())
    );

    // Back to normal the same way.
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::SpeedRequest {
            speed: GameSpeed::NORMAL,
        }),
    );
    step_both(&mut host, &mut peer, 20);
    assert_eq!(speed(&host), GameSpeed::NORMAL);
    assert_eq!(speed(&peer), GameSpeed::NORMAL);
}

#[test]
fn pause_and_speed_proposed_for_same_tick_both_apply() {
    // The two changes are scheduled from the same tick, so they land on the same
    // effective tick. Each is kept in its own store; sharing one keyed by tick
    // would let whichever arrived first be evicted by the other.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 6);

    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::SpeedRequest {
            speed: double_speed(),
        }),
    );
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: true }),
    );

    step_both(&mut host, &mut peer, 20);
    assert!(is_paused(&host) && is_paused(&peer), "both paused");
    assert_eq!(speed(&host), double_speed(), "host kept the speed change");
    assert_eq!(speed(&peer), double_speed(), "peer kept the speed change");
}

#[test]
fn speed_change_requested_while_paused_applies_on_resume() {
    // A speed is inert while the tick is frozen, and one stamped at the frozen
    // tick would race a concurrent resume — a node that moved first would
    // discard it as stale, leaving the speeds divergent for good. So the change
    // pends past the pause and applies once the resumed loop reaches its tick,
    // on every node alike.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 6);
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: true }),
    );
    step_both(&mut host, &mut peer, 20);
    assert!(is_paused(&host) && is_paused(&peer));

    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::SpeedRequest {
            speed: double_speed(),
        }),
    );
    step_both(&mut host, &mut peer, 20);
    assert_eq!(speed(&host), GameSpeed::NORMAL, "inert while frozen");
    assert_eq!(speed(&peer), GameSpeed::NORMAL);

    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: false }),
    );
    step_both(&mut host, &mut peer, 20);
    assert!(!is_paused(&host) && !is_paused(&peer), "both resumed");
    assert_eq!(speed(&host), double_speed(), "and the speed followed");
    assert_eq!(speed(&peer), double_speed());
}

#[test]
fn frame_for_player_without_slot_is_ignored() {
    // A frame naming a player no slot seats is corrupt, not input — recording it
    // would index past the per-player stores and panic the receiver.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 6);
    assert_eq!(utils::tick(&host), 6, "six steps, six ticks");

    // A tick still ahead of both nodes, so the frame would be recorded rather
    // than dismissed as late.
    peer.world_mut()
        .get_non_send_resource_mut::<NetworkSession>()
        .expect("network session")
        .0
        .broadcast_frames(vec![PlayerFrame::idle(200, 8)])
        .expect("broadcast");

    step_both(&mut host, &mut peer, 10);

    for (name, app) in [("host", &host), ("peer", &peer)] {
        let session = app.world().resource::<GameSession>();
        assert_eq!(session.result(), None, "{name} played on");
        assert!(!session.is_blocked(), "{name} never blocked");
        assert_eq!(utils::tick(app), 16, "{name} reached the sixteenth tick");
    }
    assert_eq!(
        checksum::state_checksum(host.world()),
        checksum::state_checksum(peer.world()),
        "and the two stayed bit-identical",
    );
}

#[test]
fn drop_at_for_player_without_slot_is_ignored() {
    // A drop naming a player no slot seats is a corrupt message, not a drop —
    // applying it would index past the session's slots and panic the receiver.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 6);

    let ahead = utils::tick(&host) + 5;
    send_control(
        &mut host,
        ControlMessage::InGame(InGameMessage::DropAt {
            player: 200,
            tick: ahead,
        }),
    );

    step_both(&mut host, &mut peer, 10);
    let session = peer.world().resource::<GameSession>();
    assert_eq!(session.result(), None, "the corrupt drop changed nothing");
    assert_eq!(session.dropped_players().count(), 0, "nobody was dropped");
    assert_eq!(
        utils::tick(&host),
        utils::tick(&peer),
        "the peers are still in lockstep",
    );
    // Past the drop's named tick, so the ignoring is proven where the drop would
    // have applied.
    assert_eq!(
        utils::tick(&host),
        16,
        "the host reached the sixteenth tick"
    );
    assert_eq!(utils::tick(&peer), 16, "and so did the peer");
}

#[test]
fn any_player_can_resume_pause_under_peer_authority() {
    // Pause and resume are both stamped at the frozen tick, so they land on the
    // same key in the pending store. The resume must be taken whoever sends it —
    // ranking it against the pause by player id would leave the session frozen
    // for everyone the collision rule puts after the pauser.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::new((0..2).collect());
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
    step_both(&mut host, &mut peer, 4);

    // Player 0 pauses — the lower id, so its proposal wins any comparison.
    host.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(true);
    step_both(&mut host, &mut peer, 12);
    assert!(is_paused(&host) && is_paused(&peer), "both paused");
    // Requested at tick 4 and stamped CONTROL_DELAY ahead, so both freeze at 8.
    assert_eq!(utils::tick(&host), 8, "the host froze at the agreed tick");
    assert_eq!(utils::tick(&peer), 8, "and the peer at the same one");

    // Player 1 — the higher id — resumes.
    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(false);
    step_both(&mut host, &mut peer, 12);

    assert!(!is_paused(&host) && !is_paused(&peer), "both resumed");
    // The resume commits a step apart on the two nodes — the bounded skew
    // lockstep permits — so they sit one tick apart afterwards.
    assert_eq!(utils::tick(&host), 19, "the host played on");
    assert_eq!(utils::tick(&peer), 20, "the peer a tick further");
    // State comparisons need a shared tick.
    align_ticks(&mut host, &mut peer);
    assert_eq!(
        checksum::state_checksum(host.world()),
        checksum::state_checksum(peer.world()),
        "and they stayed bit-identical across the pause",
    );
}

#[test]
fn duplicate_pause_at_on_frozen_tick_is_not_re_learned() {
    // A mesh floods what it learns, so a duplicate of an applied change keeps
    // arriving. While paused the tick is frozen, so the entry that was applied
    // is still the current one: it must be recognised as already applied, not
    // re-learned and re-forwarded on every step for the whole pause.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::new((0..2).collect());
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
    step_both(&mut host, &mut peer, 4);

    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(true);
    step_both(&mut host, &mut peer, 12);
    assert!(is_paused(&host) && is_paused(&peer), "both paused");
    let frozen = utils::tick(&host);

    // Re-deliver the very change the receiver already applied at the frozen
    // tick — a flooded duplicate. It must change nothing and, crucially, must
    // not be forwarded onward again.
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::PauseAt {
            proposer: Proposer::Player(1),
            tick: frozen,
            paused: true,
        }),
    );
    step_both(&mut host, &mut peer, 10);

    assert!(is_paused(&host), "still paused");
    assert_eq!(utils::tick(&host), frozen, "and still frozen");
}

#[test]
fn pause_request_reaching_only_client_is_ignored() {
    // Only the deciding node acts on a bare request. A client that both queued
    // it and re-sent it from the non-host arm would amplify a forged request
    // into a flood between peers; here the request reaches client 1 alone (it
    // has no link to client 2's would-be audience), so nothing may come of it.
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (1, 2)])
        .into_iter()
        .map(|t| utils::net_app(t, 3))
        .collect();
    step_all(&mut apps, 6);

    // Client 2 forges a request; its only link is to client 1, never the host.
    send_control(
        &mut apps[2],
        ControlMessage::InGame(InGameMessage::PauseRequest { paused: true }),
    );
    step_all(&mut apps, 20);

    for (i, app) in apps.iter().enumerate() {
        assert!(
            !is_paused(app),
            "node {i} paused on a request only a client saw",
        );
    }
}

#[test]
fn speed_at_from_client_is_refused_under_host_authority() {
    // Under host authority only the host node announces changes; a forged
    // authoritative message from a client must not steer the host's cadence
    // away from everybody else's.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 6);

    let ahead = utils::tick(&peer) + 10;
    send_control(
        &mut peer,
        ControlMessage::InGame(InGameMessage::SpeedAt {
            proposer: Proposer::Player(1),
            tick: ahead,
            speed: double_speed(),
        }),
    );

    step_both(&mut host, &mut peer, 20);
    assert_eq!(
        speed(&host),
        GameSpeed::NORMAL,
        "the forgery changed nothing"
    );
    assert_eq!(speed(&peer), GameSpeed::NORMAL);
}

#[test]
fn frames_from_keeping_up_peer_arrive_ahead_of_tick_that_needs_them() {
    // A peer in lockstep sends each frame SYNC_LATENCY ticks ahead of the tick
    // that will execute it, so its frames land with room to spare.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);

    step_both(&mut host, &mut peer, 12);

    // `step_both` advances the two apps in strict alternation, so whichever
    // steps first each round learns its peer's frame a tick later than the peer
    // learns its own: the full lead on one side, a tick less on the other. Both
    // are keeping up — what matters is that the margin stays above zero, since a
    // margin of zero is a frame that arrived only just in time.
    assert_eq!(
        host.world().resource::<FrameMargins>().of(1),
        Some(FixedU64::ONE)
    );
    assert_eq!(
        peer.world().resource::<FrameMargins>().of(0),
        Some(FixedU64::from_num(SYNC_LATENCY)),
    );
    assert_eq!(
        host.world().resource::<FrameMargins>().of(0),
        None,
        "a node's own frames never arrive over the net",
    );
    let tick = utils::tick(&host);
    assert_eq!(
        host.world().resource::<FrameMargins>().tightest(tick),
        Some(FixedU64::ONE)
    );
}

#[test]
fn margin_of_peer_that_stopped_sending_is_forgotten() {
    // Whatever margin a departed peer left behind, it is no longer a statement
    // about keeping up — holding the cadence down on it for the rest of the match
    // would be a bug.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    step_both(&mut host, &mut peer, 12);
    let heard = utils::tick(&host);
    assert_eq!(
        host.world().resource::<FrameMargins>().tightest(heard),
        Some(FixedU64::ONE)
    );

    assert_eq!(
        host.world()
            .resource::<FrameMargins>()
            .tightest(heard + 100),
        None,
        "a margin nobody refreshed stops constraining the cadence",
    );
    assert_eq!(
        host.world().resource::<FrameMargins>().of(1),
        Some(FixedU64::ONE),
        "the last value is still readable, it just no longer counts",
    );
}

#[test]
fn capacity_reports_reach_peers_and_fold_to_slowest() {
    // Each node publishes what it can hold and folds what it hears; the fold is
    // a minimum, so the slowest peer sets the ceiling for everybody.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    utils::set_tick_cost(&mut host, CHEAP_TICK);
    utils::set_tick_cost(&mut peer, EXPENSIVE_TICK);

    step_both(&mut host, &mut peer, 45);

    let tick = utils::tick(&host);
    assert_eq!(
        host.world().resource::<PeerCapacities>().tightest(tick),
        Some(capacity_of(EXPENSIVE_TICK)),
        "the host heard the struggling peer",
    );
    assert_eq!(
        peer.world().resource::<PeerCapacities>().tightest(tick),
        Some(capacity_of(EXPENSIVE_TICK)),
        "and the peer heard the host's report, which folds the peer's own struggle back in",
    );
    assert!(
        capacity_of(EXPENSIVE_TICK) < capacity_of(CHEAP_TICK),
        "the expensive tick is the one that constrains",
    );
}

#[test]
fn capacity_minimum_crosses_star_through_host_fold() {
    // Star control links: clients 1 and 2 reach only the host, so client 1's
    // report on its own can never reach client 2. The host folding what it heard
    // into its own report is what carries the group's minimum across.
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)])
        .into_iter()
        .map(|t| utils::net_app(t, 3))
        .collect();
    utils::set_tick_cost(&mut apps[0], CHEAP_TICK);
    utils::set_tick_cost(&mut apps[1], EXPENSIVE_TICK);
    utils::set_tick_cost(&mut apps[2], CHEAP_TICK);

    step_all(&mut apps, 45);

    let tick = utils::tick(&apps[2]);
    assert_eq!(
        apps[2].world().resource::<PeerCapacities>().tightest(tick),
        Some(capacity_of(EXPENSIVE_TICK)),
        "client 2 hears the struggling client only through the host's fold",
    );
}

#[test]
fn stale_capacity_report_stops_constraining() {
    // A peer that goes quiet must not hold the game down forever, so its last
    // report ages out.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
    utils::set_tick_cost(&mut peer, EXPENSIVE_TICK);
    step_both(&mut host, &mut peer, 25);
    let heard = utils::tick(&host);
    assert_eq!(
        host.world().resource::<PeerCapacities>().tightest(heard),
        Some(capacity_of(EXPENSIVE_TICK)),
    );

    // Read the same store far enough in the future and the report is forgotten.
    assert_eq!(
        host.world()
            .resource::<PeerCapacities>()
            .tightest(heard + 1000),
        None,
    );
}

#[test]
fn gone_peer_is_dropped_and_rest_continue_in_lockstep() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| utils::net_app(t, 3))
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
        checksum::state_checksum(apps[0].world()),
        checksum::state_checksum(apps[1].world()),
        "the two survivors must stay bit-identical after the drop",
    );
}

#[test]
fn two_peer_disconnect_aborts_remaining_peer() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
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
        PlayerSlot::occupied(
            2,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            None,
            None,
        ),
    ];
    let authority = Authority::Host {
        ai_hosting: AiHosting::Replicated,
    };
    let mut host = utils::net_app_with_slots(a, roster.clone(), authority, slots.clone());
    let mut peer = utils::net_app_with_slots(b, roster, authority, slots);
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
        .map(|t| utils::net_app(t, 3))
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
        checksum::state_checksum(apps[0].world()),
        checksum::state_checksum(apps[2].world()),
        "a recovered peer stays in lockstep with the rest",
    );
}

#[test]
fn diverging_one_peer_trips_desync() {
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);

    step_both(&mut host, &mut peer, 5);

    // Force a divergence: spawn an entity on the host OUTSIDE the lockstep command
    // pipeline, so only the host's state (and checksum) changes.
    utils::create_entity(host.world_mut(), "soldier", utils::pos(3, 3), Some(0))
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
    let mut app =
        utils::net_app_with_roster(a, Roster::from_slots(vec![Some(0), Some(1), Some(99)]));
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
    let mut host = utils::net_app_with_roster(a, roster.clone());
    let mut peer = utils::net_app_with_roster(b, roster);
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
        checksum::state_checksum(host.world_mut()),
        checksum::state_checksum(peer.world_mut()),
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
        .map(|t| utils::net_app(t, 3))
        .collect();

    step_all(&mut apps, 6);
    let already_run = utils::tick(&apps[1]) - 2;

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
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;

    step_both(&mut host, &mut peer, 60);

    assert!(host.world().resource::<GameSession>().is_player_dropped(2));
    assert!(peer.world().resource::<GameSession>().is_player_dropped(2));
    align_ticks(&mut host, &mut peer);
    let tick = host.world().resource::<GameSession>().tick();
    assert!(tick > 3, "the survivors kept playing past the drop");
    assert_eq!(
        checksum::state_checksum(host.world_mut()),
        checksum::state_checksum(peer.world_mut()),
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
        PlayerSlot::environment(3, AiVision::Filtered),
    ];
    let mut host = utils::net_app_with_slots(a, roster.clone(), Authority::Peers, slots.clone());
    let mut peer = utils::net_app_with_slots(b, roster, Authority::Peers, slots);
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
        checksum::state_checksum(host.world_mut()),
        checksum::state_checksum(peer.world_mut()),
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
        .map(|t| utils::net_app_configured(t, roster.clone(), Authority::Peers))
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
        checksum::state_checksum(left[0].world_mut()),
        checksum::state_checksum(right[0].world_mut()),
    );
}

#[test]
fn losing_control_link_to_host_aborts_client() {
    // Under host authority a client whose control link to the host died can
    // no longer be steered: no DropAt or PauseAt will ever arrive, however
    // healthy the gameplay traffic looks.
    let (a, b) = LoopbackTransport::pair();
    let mut host = utils::net_app(a, 2);
    let mut peer = utils::net_app(b, 2);
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
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
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
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
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
        .map(|t| utils::net_app(t, 3))
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
        .map(|(t, authority)| {
            utils::net_app_configured(t, Roster::new((0..3).collect()), authority)
        })
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
        checksum::state_checksum(left[1].world_mut()),
        checksum::state_checksum(right[0].world_mut()),
    );
}

#[test]
fn manual_policy_holds_drop_until_game_approves() {
    // The deciding host runs the manual policy: the stall is surfaced but
    // nobody is dropped past the grace window until the game (a wait dialog,
    // some day) approves the player.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::from_slots(vec![Some(0), Some(1), Some(99)]);
    let mut host = utils::net_app_with_roster(a, roster.clone());
    let mut peer = utils::net_app_with_roster(b, roster);
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
    let mut host = utils::net_app_configured(a, Roster::new((0..2).collect()), Authority::Peers);
    let mut peer = utils::net_app_configured(b, Roster::new((0..2).collect()), Authority::Peers);
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
        checksum::state_checksum(host.world_mut()),
        checksum::state_checksum(peer.world_mut()),
    );
}

#[test]
fn stale_pause_proposal_does_not_resurrect_after_its_tick_passed() {
    // Pause and resume normally, then replay a copy of the original pause —
    // a flooded duplicate arriving long after the change was applied and
    // discarded. It must be ignored, not re-applied.
    let (a, b) = LoopbackTransport::pair();
    let roster = Roster::new((0..2).collect());
    let mut host = utils::net_app_configured(a, roster.clone(), Authority::Peers);
    let mut peer = utils::net_app_configured(b, roster, Authority::Peers);
    step_both(&mut host, &mut peer, 4);

    peer.world_mut()
        .resource_mut::<ferrets_bevy_plugin::PauseIntent>()
        .0 = Some(true);
    step_both(&mut host, &mut peer, 12);
    let frozen = utils::tick(&host);
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
            proposer: Proposer::Player(0),
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
    let mut host = utils::net_app_with_roster(a, roster.clone());
    let mut peer = utils::net_app_with_roster(b, roster);
    host.add_plugins(ReplayPlugin);
    host.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;
    peer.world_mut().resource_mut::<DropConfig>().timeout_steps = 10;

    let buffer = utils::record_into(
        &mut host,
        &utils::skirmish_header(utils::human_slots(3), FinishPolicy::Endless),
    );

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
    let mut playback = utils::make_app(utils::human_slots(3));
    playback.add_plugins(ReplayPlugin);
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(utils::harness_soldier());
        registry.validate();
    }
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);
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
    let mut host = utils::net_app_with_slots(a, roster.clone(), authority, teamed_human_slots());
    let mut peer = utils::net_app_with_slots(b, roster, authority, teamed_human_slots());
    host.add_plugins(ReplayPlugin);
    // Starting units, like a game's setup would place them — spawned identically
    // on every node (not recorded as input, so playback re-creates them the same
    // way).
    for app in [&mut host, &mut peer] {
        spawn_starting_units(app);
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
    }

    let buffer = utils::record_into(
        &mut host,
        &utils::skirmish_header(
            teamed_human_slots(),
            FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        ),
    );

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
        .set_finish_policy(FinishPolicy::LastStanding {
            elimination: EliminationScope::Player,
        });
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(utils::harness_soldier());
        registry.register(utils::harness_base());
        registry.validate();
    }
    spawn_starting_units(&mut playback);
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);
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
    let mut host = utils::net_app_with_roster(a, roster.clone());
    let mut peer = utils::net_app_with_roster(b, roster);
    host.add_plugins(ReplayPlugin);
    let mut ids = Vec::new();
    for app in [&mut host, &mut peer] {
        ids.push(spawn_lone_winner_lineup(app));
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 5;
    }
    let (attacker, target) = ids[0];

    let buffer = utils::record_into(
        &mut host,
        &utils::skirmish_header(
            utils::human_slots(3),
            FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        ),
    );

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
    let mut playback = utils::make_app(utils::human_slots(3));
    playback.add_plugins(ReplayPlugin);
    playback
        .world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding {
            elimination: EliminationScope::Player,
        });
    {
        let mut registry = playback.world_mut().resource_mut::<ContentRegistry>();
        registry.register(utils::harness_soldier());
        registry.register(utils::harness_base());
        registry.validate();
    }
    spawn_lone_winner_lineup(&mut playback);
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);
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
    let buffer = utils::record_into(
        &mut record_app,
        &utils::skirmish_header(
            utils::human_slots(2),
            FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        ),
    );

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
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);

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
        .map(|t| utils::net_app(t, 3))
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        ids.push(spawn_ffa_combatants(app));
        let mut session = app.world_mut().resource_mut::<GameSession>();
        session.set_finish_policy(FinishPolicy::LastStanding {
            elimination: EliminationScope::Player,
        });
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
        utils::tick(&apps[0]) > utils::tick(&apps[2]) + 30,
        "the survivors kept ticking after node 2 froze",
    );
    let (left, right) = apps.split_at_mut(1);
    align_ticks(&mut left[0], &mut right[0]);
    assert_eq!(
        checksum::state_checksum(left[0].world()),
        checksum::state_checksum(right[0].world()),
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
    let buffer = utils::record_into(
        &mut record_app,
        &utils::skirmish_header(
            utils::human_slots(3),
            FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        ),
    );

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
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);

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
// ─── Spectators ─────────────────────────────────────────────────────────────────
//

#[test]
fn spectating_defeated_node_stays_in_lockstep_with_survivors() {
    // The counterpart of `eliminated_player_node_freezing_does_not_stall_survivors`:
    // the same three-way free-for-all, but node 2 runs `Spectate`. Its defeat
    // finishes nothing — the node keeps ticking in lockstep, bit-identical to
    // the survivors, with no result of its own.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| utils::net_app(t, 3))
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        ids.push(spawn_ffa_combatants(app));
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
    }
    let (attacker, target) = ids[0];
    apps[2]
        .world_mut()
        .resource_mut::<GameSession>()
        .set_defeat_conduct(DefeatConduct::Spectate);

    utils::select(&mut apps[0], attacker);
    utils::push_command(
        &mut apps[0],
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
    step_all(&mut apps, 80);

    // Node 2 learned of its defeat and watched on instead of freezing...
    {
        let session = apps[2].world().resource::<GameSession>();
        assert_eq!(session.result(), None);
        assert!(session.is_player_eliminated(2));
        assert!(!session.local_plays());
    }
    // ...in step with the survivors, not trailing them.
    assert!(utils::tick(&apps[2]) + 2 >= utils::tick(&apps[0]));
    let (left, right) = apps.split_at_mut(2);
    align_ticks(&mut left[0], &mut right[0]);
    assert_eq!(
        checksum::state_checksum(left[0].world()),
        checksum::state_checksum(right[0].world()),
        "the spectating node must stay bit-identical to the survivors",
    );
}

#[test]
fn spectating_eliminated_host_keeps_relaying_for_survivors() {
    // Star links: clients 1 and 2 reach only the host, so every frame between
    // them relays through it. The HOST's player is eliminated — the boundary
    // that used to end the game for everyone — but under `Spectate` its node
    // keeps simulating, relaying, and deciding, and the survivors play the
    // match to its shared verdict.
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)])
        .into_iter()
        .map(|t| utils::net_app(t, 3))
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
        ids.push(spawn_host_elimination_lineup(app));
    }
    let (attacker, host_base, last_base) = ids[0];
    apps[0]
        .world_mut()
        .resource_mut::<GameSession>()
        .set_defeat_conduct(DefeatConduct::Spectate);

    // Player 1 (a client) destroys the host's base.
    utils::select(&mut apps[1], attacker);
    utils::push_command(
        &mut apps[1],
        PlayerCommand::SendToEntity {
            target: host_base,
            flush: true,
        },
    );
    step_all(&mut apps, 80);

    for app in &apps {
        let session = app.world().resource::<GameSession>();
        assert!(session.is_player_eliminated(0));
        assert_eq!(session.result(), None, "nobody aborted, nobody finished");
    }

    // The match plays on through the spectating host's relay to its verdict.
    utils::push_command(
        &mut apps[1],
        PlayerCommand::SendToEntity {
            target: last_base,
            flush: true,
        },
    );
    step_all(&mut apps, 200);

    for app in &apps {
        assert_eq!(
            app.world().resource::<GameSession>().result(),
            Some(GameResult::Victory {
                winner: Winner::Player(1)
            }),
        );
    }
}

#[test]
fn observer_node_stays_synced_and_stalls_nobody() {
    // A mesh game with a third connected peer holding no slot — an observer:
    // its node simulates in lockstep from the broadcast frames and receives
    // the shared verdict, with no local player at all.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| {
            observer_net_app(
                t,
                Authority::Host {
                    ai_hosting: AiHosting::Replicated,
                },
            )
        })
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
        ids.push(spawn_combatants(app));
    }
    let (attacker, enemy_base) = ids[0];
    {
        let session = apps[2].world().resource::<GameSession>();
        assert_eq!(session.local_player(), None);
        assert!(!session.local_plays());
    }

    utils::select(&mut apps[0], attacker);
    utils::push_command(
        &mut apps[0],
        PlayerCommand::SendToEntity {
            target: enemy_base,
            flush: true,
        },
    );
    step_all(&mut apps, 90);

    // Everyone — the observer included — reached the same shared verdict...
    for app in &apps {
        assert_eq!(
            app.world().resource::<GameSession>().result(),
            Some(GameResult::Victory {
                winner: Winner::Player(0)
            }),
        );
    }
    let (left, right) = apps.split_at_mut(2);
    align_ticks(&mut left[0], &mut right[0]);
    assert_eq!(
        checksum::state_checksum(left[0].world()),
        checksum::state_checksum(right[0].world()),
        "the observer must stay bit-identical to the combatants",
    );
}

#[test]
fn frozen_observer_node_stalls_and_drops_nobody() {
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| {
            observer_net_app(
                t,
                Authority::Host {
                    ai_hosting: AiHosting::Replicated,
                },
            )
        })
        .collect();
    for app in &mut apps {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 3;
    }
    step_all(&mut apps, 6);

    // The observer's node freezes; the combatants play far past it with no
    // stall resolution of any kind.
    step_some(&mut apps, &[0, 1], 40);

    for combatant in [0, 1] {
        let session = apps[combatant].world().resource::<GameSession>();
        assert_eq!(session.result(), None);
        assert!(!session.is_player_dropped(0));
        assert!(!session.is_player_dropped(1));
        assert!(utils::tick(&apps[combatant]) > utils::tick(&apps[2]) + 30);
    }
}

#[test]
fn observer_host_carries_game_for_combatant_clients() {
    // Star links again, with the HOST node holding no slot at all — a
    // caster's setup: the two combatants are clients who reach each other only
    // through the watching host.
    let roster = Roster::from_slots(vec![Some(1), Some(2)]);
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ];
    let authority = Authority::Host {
        ai_hosting: AiHosting::Replicated,
    };
    let mut apps: Vec<App> = LoopbackTransport::partial_mesh(3, [(0, 1), (0, 2)])
        .into_iter()
        .map(|t| utils::net_app_with_slots(t, roster.clone(), authority, slots.clone()))
        .collect();
    let mut ids = Vec::new();
    for app in &mut apps {
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
        ids.push(spawn_combatants(app));
    }
    let (attacker, enemy_base) = ids[0];
    {
        let session = apps[0].world().resource::<GameSession>();
        assert_eq!(session.local_player(), None);
        assert!(!session.local_plays());
    }

    // Player 0 plays on the node behind peer 1.
    utils::select(&mut apps[1], attacker);
    utils::push_command(
        &mut apps[1],
        PlayerCommand::SendToEntity {
            target: enemy_base,
            flush: true,
        },
    );
    step_all(&mut apps, 90);

    for app in &apps {
        assert_eq!(
            app.world().resource::<GameSession>().result(),
            Some(GameResult::Victory {
                winner: Winner::Player(0)
            }),
        );
    }
}

#[test]
fn drop_consensus_with_observer_commits_on_every_node() {
    // Peer authority: a combatant goes silent, and the drop commits by the
    // unanimity of the LIVE COMBATANTS — an observer holds no vote at all,
    // yet its node applies the same drop from the flooded votes.
    let roster = Roster::new((0..3).collect());
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
        PlayerSlot::occupied(2, PlayerType::Human, None, None),
    ];
    let mut apps: Vec<App> = LoopbackTransport::mesh(4)
        .into_iter()
        .map(|t| utils::net_app_with_slots(t, roster.clone(), Authority::Peers, slots.clone()))
        .collect();
    for app in &mut apps {
        app.world_mut().resource_mut::<DropConfig>().timeout_steps = 3;
    }

    step_all(&mut apps, 6);
    // Player 1's node goes silent; everyone else — observer included — plays on.
    step_some(&mut apps, &[0, 2, 3], 40);

    for alive in [0, 2, 3] {
        let session = apps[alive].world().resource::<GameSession>();
        assert!(session.is_player_dropped(1), "node {alive} missed the drop");
        assert_eq!(session.result(), None);
    }
}

#[test]
fn game_recorded_by_observer_node_replays_identically() {
    // The observer's node holds no slot, yet it simulates the whole game from
    // the broadcast frames — so its recording carries every player's committed
    // input and replays checksum-clean to the same verdict.
    let mut apps: Vec<App> = LoopbackTransport::mesh(3)
        .into_iter()
        .map(|t| {
            observer_net_app(
                t,
                Authority::Host {
                    ai_hosting: AiHosting::Replicated,
                },
            )
        })
        .collect();
    for app in &mut apps {
        app.world_mut()
            .resource_mut::<GameSession>()
            .set_finish_policy(FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            });
    }
    apps[2].add_plugins(ReplayPlugin);
    let buffer = utils::record_into(
        &mut apps[2],
        &utils::skirmish_header(
            utils::human_slots(2),
            FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        ),
    );
    let mut ids = Vec::new();
    for app in &mut apps {
        ids.push(spawn_combatants(app));
    }
    let (attacker, enemy_base) = ids[0];

    utils::select(&mut apps[0], attacker);
    utils::push_command(
        &mut apps[0],
        PlayerCommand::SendToEntity {
            target: enemy_base,
            flush: true,
        },
    );
    step_all_recording(&mut apps, 90);
    assert_eq!(
        apps[2].world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
    );

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");
    let mut playback = two_player_app();
    playback.add_plugins(ReplayPlugin);
    spawn_combatants(&mut playback);
    ferrets_bevy_plugin::replay::playback::install_per_game(playback.world_mut(), replay);

    for _ in 0..120 {
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
    assert_eq!(watched.mismatch(), None);
    assert_eq!(
        playback.world().resource::<GameSession>().result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        }),
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

/// A local two-player app with the armed soldier and the base, `LastStanding`,
/// and the session started — what an observer's recording rebuilds into.
fn two_player_app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(utils::harness_soldier());
        registry.register(utils::harness_base());
        registry.validate();
    }
    let mut session = app.world_mut().resource_mut::<GameSession>();
    session.set_finish_policy(FinishPolicy::LastStanding {
        elimination: EliminationScope::Player,
    });
    session.start();
    app
}

/// Advances every app one fixed tick each — plus `FixedLast`, so a recording
/// node captures each completed tick — for `ticks` ticks.
fn step_all_recording(apps: &mut [App], ticks: u32) {
    for _ in 0..ticks {
        for app in apps.iter_mut() {
            app.world_mut().run_schedule(FixedUpdate);
            app.world_mut().run_schedule(FixedLast);
        }
    }
}

/// Like [`utils::net_app`], for a two-player game with a third connected peer
/// that holds no slot at all — an observer: its node receives every broadcast
/// and runs with no local player.
fn observer_net_app(transport: LoopbackTransport, authority: Authority) -> App {
    let roster = Roster::new((0..2).collect());
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ];
    utils::net_app_with_slots(transport, roster, authority, slots)
}

/// Sets up the eliminated-host lineup: every player's base — the host's within
/// the attacker's reach — plus player 1's soldier beside it. Spawned in a fixed
/// order so ids match across every node. Returns the attacker, the host's base,
/// and player 2's base [`SimulationId`]s.
fn spawn_host_elimination_lineup(app: &mut App) -> (SimulationId, SimulationId, SimulationId) {
    let world = app.world_mut();
    let (_, host_base) =
        utils::create_entity(world, "base", utils::pos(6, 5), Some(0)).expect("host base");
    utils::create_entity(world, "base", utils::pos(25, 25), Some(1)).expect("player 1 base");
    let (_, last_base) =
        utils::create_entity(world, "base", utils::pos(5, 8), Some(2)).expect("player 2 base");
    let (_, attacker) =
        utils::create_entity(world, "soldier", utils::pos(5, 5), Some(1)).expect("attacker");
    (attacker, host_base, last_base)
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

/// Two tick costs either side of what the nominal cadence affords.
const CHEAP_TICK: FixedU64 = FixedU64::lit("1");
const EXPENSIVE_TICK: FixedU64 = FixedU64::lit("65");

/// The capacity a node measuring `exec_millis` per tick reports.
fn capacity_of(exec_millis: FixedU64) -> GameSpeed {
    GameSpeed::new(ferrets_bevy_plugin::sustainable_factor(
        exec_millis,
        NOMINAL_MILLIS,
    ))
}

fn speed(app: &App) -> GameSpeed {
    app.world().resource::<GameSession>().speed()
}

/// Twice the nominal cadence — a speed distinguishable from `NORMAL`.
fn double_speed() -> GameSpeed {
    GameSpeed::new(FixedU64::from_num(2))
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
    utils::create_entity(world, "base", utils::pos(5, 5), Some(0)).expect("player 0 base");
    utils::create_entity(world, "base", utils::pos(10, 10), Some(1)).expect("player 1 base");
    utils::create_entity(world, "base", utils::pos(20, 20), Some(2)).expect("phantom base");
}

/// A fresh two-player app with a combat-capable soldier, `LastStanding`, and the
/// session started — the setup a last-standing win is recorded and replayed on.
fn combat_victory_app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(utils::harness_soldier());
        registry.register(utils::harness_base());
        registry.validate();
    }
    let mut session = app.world_mut().resource_mut::<GameSession>();
    session.set_finish_policy(FinishPolicy::LastStanding {
        elimination: EliminationScope::Player,
    });
    session.start();
    app
}

/// A fresh three-player free-for-all app with the armed soldier and the base,
/// `LastStanding`, and the session started — the setup a mid-game elimination
/// is recorded and replayed on.
fn ffa_elimination_app() -> App {
    let mut app = utils::make_app(utils::human_slots(3));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(utils::harness_soldier());
        registry.register(utils::harness_base());
        registry.validate();
    }
    let mut session = app.world_mut().resource_mut::<GameSession>();
    session.set_finish_policy(FinishPolicy::LastStanding {
        elimination: EliminationScope::Player,
    });
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
    utils::create_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    utils::create_entity(world, "base", utils::pos(25, 25), Some(1)).expect("player 1 base");
    let (_, target) =
        utils::create_entity(world, "base", utils::pos(6, 5), Some(2)).expect("player 2 base");
    let (_, attacker) =
        utils::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
    (attacker, target)
}

/// Sets up the lone-winner lineup: a base per slot — the phantom slot 2
/// included — plus player 0's soldier next to player 1's base. Spawned in a
/// fixed order so ids — and thus the state checksum — match across every node
/// and the replay. Returns the attacker and player 1's base [`SimulationId`]s.
fn spawn_lone_winner_lineup(app: &mut App) -> (SimulationId, SimulationId) {
    let world = app.world_mut();
    utils::create_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    let (_, target) =
        utils::create_entity(world, "base", utils::pos(6, 5), Some(1)).expect("player 1 base");
    utils::create_entity(world, "base", utils::pos(25, 25), Some(2)).expect("phantom base");
    let (_, attacker) =
        utils::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
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
    utils::create_entity(world, "base", utils::pos(5, 8), Some(0)).expect("player 0 base");
    let (_, attacker) =
        utils::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).expect("attacker");
    let (_, enemy_base) =
        utils::create_entity(world, "base", utils::pos(6, 5), Some(1)).expect("enemy base");
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
                .filter(|&id| Some(id) != local)
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
