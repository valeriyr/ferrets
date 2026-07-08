//! The committed-input store: first-write-wins recording and readiness
//! relative to the players a tick requires.

use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::input::{InputFrames, PlayerFrame};

#[test]
fn tick_is_not_ready_while_required_player_is_missing() {
    let mut frames = InputFrames::new(2);
    frames.push_frame(stop_frame(0, 2));

    assert!(frames.ready_commands(2, &[0, 1]).is_none());
}

#[test]
fn ready_commands_returns_each_required_players_commands() {
    let mut frames = InputFrames::new(2);
    frames.push_frame(stop_frame(0, 2));
    frames.push_frame(PlayerFrame::idle(1, 2));

    let ready = frames.ready_commands(2, &[0, 1]).expect("tick ready");

    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0], (0, [PlayerCommand::Stop].as_slice()));
    assert_eq!(ready[1], (1, [].as_slice()));
}

#[test]
fn frames_from_players_not_required_are_ignored() {
    let mut frames = InputFrames::new(2);
    frames.push_frame(stop_frame(0, 2));
    // Player 1 delivered a real frame too, but the tick does not require it
    // (e.g. the player was dropped): it neither holds the tick up nor appears
    // in the result — only the required player's commands come back.
    frames.push_frame(stop_frame(1, 2));

    let ready = frames.ready_commands(2, &[0]).expect("tick ready");

    assert_eq!(ready, vec![(0, [PlayerCommand::Stop].as_slice())]);
}

#[test]
fn redundant_copy_of_recorded_frame_is_no_op() {
    let mut frames = InputFrames::new(2);
    frames.push_frame(stop_frame(0, 2));
    // An identical copy is the only redundancy the contract permits (the
    // resend window, relay hops); a differing repeat is a determinism bug and
    // panics instead — see the test below. So what this pins is that the copy
    // does not accumulate: the commands are recorded once, not appended twice.
    frames.push_frame(stop_frame(0, 2));

    let ready = frames.ready_commands(2, &[0]).expect("tick ready");
    assert_eq!(ready, vec![(0, [PlayerCommand::Stop].as_slice())]);
}

#[test]
#[should_panic(expected = "conflicting input for player 0: input is immutable once recorded")]
fn differing_repeat_for_recorded_frame_panics() {
    let mut frames = InputFrames::new(2);
    frames.push_frame(stop_frame(0, 2));
    frames.push_frame(PlayerFrame::idle(0, 2));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

fn stop_frame(player: u8, tick: u32) -> PlayerFrame {
    PlayerFrame {
        player,
        tick,
        commands: vec![PlayerCommand::Stop],
    }
}
