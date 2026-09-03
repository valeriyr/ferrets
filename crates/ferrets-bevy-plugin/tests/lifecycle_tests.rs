//! Installing and tearing down a game's engine state: whatever a previous game
//! left behind, entering clears it, and leaving removes what the entry paths
//! installed.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{
    DropIntent, FrameMargins, NetworkActive, NetworkSession, PauseIntent, PeerCapacities, Seek,
    SpeedIntent, Stall, StallInfo, StallVotes, Step, TickPacing,
};
use ferrets_math::FixedU64;
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_simulation::session::{GameSession, game_speed::GameSpeed};

#[test]
fn installing_game_resources_clears_last_games_state() {
    // A game's entry path should not have to know which of the engine's
    // resources are per-game: whatever the previous one left behind, starting a
    // game clears it. Stale votes in particular would let one game's
    // observations count toward a drop in the next.
    let (a, _b) = LoopbackTransport::pair();
    let mut app = utils::net_app(a, 2);
    {
        let world = app.world_mut();
        world.resource_mut::<StallVotes>().0.insert(1, (7, vec![0]));
        world.resource_mut::<DropIntent>().0.push(1);
        world.resource_mut::<Stall>().0 = Some(StallInfo {
            tick: 7,
            missing: vec![1],
            steps: 3,
        });
        world.resource_mut::<TickPacing>().exec_millis = FixedU64::from_num(10_000);
        world.resource_mut::<PeerCapacities>().record(
            1,
            0,
            GameSpeed::new(FixedU64::from_num(0.25)),
        );
        world.insert_resource(Step);
        world.insert_resource(Seek(500));
    }

    ferrets_bevy_plugin::install_game_resources(app.world_mut());

    let world = app.world();
    assert!(world.resource::<StallVotes>().0.is_empty(), "votes");
    assert!(world.resource::<DropIntent>().0.is_empty(), "drop intent");
    assert_eq!(world.resource::<Stall>().0, None, "stall");
    assert_eq!(
        world.resource::<TickPacing>().exec_millis,
        FixedU64::ZERO,
        "tick cost",
    );
    assert!(
        !world.contains_resource::<Step>() && !world.contains_resource::<Seek>(),
        "stale step and seek requests are dropped",
    );
    assert_eq!(
        world.resource::<PeerCapacities>().tightest(0),
        None,
        "peer capacities",
    );
    assert_eq!(
        world.resource::<FrameMargins>().tightest(0),
        None,
        "margins"
    );
}

#[test]
fn tearing_down_removes_game_state() {
    // The mirror of installing: a game's exit path should not have to know
    // which engine resources a finished game leaves behind — one forgotten
    // would keep acting (a live network session keeps receiving, an installed
    // playback keeps supplying).
    let (a, _b) = LoopbackTransport::pair();
    let mut app = utils::net_app(a, 2);
    utils::create_owned(&mut app, "soldier", 10, 10, 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);

    ferrets_bevy_plugin::teardown_game_resources(app.world_mut());

    let world = app.world_mut();
    assert_eq!(utils::count_of_type(world, "soldier"), 0, "entities gone");
    assert!(
        !world.resource::<GameSession>().is_active(),
        "session reset to pending",
    );
    assert!(!world.contains_resource::<NetworkActive>());
    assert!(
        world.get_non_send_resource::<NetworkSession>().is_none(),
        "network session removed",
    );
}

#[test]
fn tearing_down_clears_unconsumed_requests() {
    // A request the finished game never consumed — one made in the last frames
    // of a networked game, whose control plane had already stopped running —
    // must not survive into the menu, where the local applier would put it on
    // the pending session and `configure` would carry it into the next game
    // (it replaces the slots, not the pause or the speed).
    let (a, _b) = LoopbackTransport::pair();
    let mut app = utils::net_app(a, 2);
    app.world_mut().resource_mut::<PauseIntent>().0 = Some(true);
    app.world_mut().resource_mut::<SpeedIntent>().0 =
        Some(GameSpeed::new(ferrets_math::FixedU64::from_num(4)));

    ferrets_bevy_plugin::teardown_game_resources(app.world_mut());

    let world = app.world();
    assert_eq!(world.resource::<PauseIntent>().0, None, "pause request");
    assert_eq!(world.resource::<SpeedIntent>().0, None, "speed request");
}
