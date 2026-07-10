//! The local frame source: one committed frame per tick, with commands issued
//! during a blocked tick buffered until the loop resumes.

mod utils;

use bevy::prelude::*;
use ferrets_simulation::{
    command::PlayerCommand,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
};

#[test]
fn commands_issued_while_blocked_flush_once_tick_resumes() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None),
        // A remote human with no frame source: the loop blocks at tick 2.
        PlayerSlot::occupied(1, PlayerType::Human, None),
    ]);
    app.world_mut().resource_mut::<GameSession>().start();
    utils::run_steps(&mut app, 3);
    assert!(app.world().resource::<GameSession>().is_blocked());

    // Clicks during the freeze must not re-open the already-committed target
    // frame (that used to trip the input-immutability assertion); they wait.
    utils::push_command(&mut app, PlayerCommand::Stop);
    utils::run_steps(&mut app, 5);
    let blocked_tick = app.world().resource::<GameSession>().tick();
    let frames = app
        .world()
        .resource::<InputFrames>()
        .frames_in_range(blocked_tick + SYNC_LATENCY, blocked_tick + SYNC_LATENCY);
    let local = frames.iter().find(|f| f.player == 0).expect("local frame");
    assert!(
        local.commands.is_empty(),
        "the frozen target stays as committed"
    );

    // Feeding the missing remote frames resumes the loop; the buffered command
    // flushes into the first fresh target.
    for tick in blocked_tick..=blocked_tick + SYNC_LATENCY {
        app.world_mut()
            .resource_mut::<InputFrames>()
            .push_frame(PlayerFrame::idle(1, tick));
    }
    utils::run_steps(&mut app, 2);
    let landed = app.world().resource::<InputFrames>().frames_in_range(
        blocked_tick + SYNC_LATENCY + 1,
        blocked_tick + SYNC_LATENCY + 1,
    );
    let local = landed.iter().find(|f| f.player == 0).expect("local frame");
    assert_eq!(local.commands, vec![PlayerCommand::Stop]);
}
