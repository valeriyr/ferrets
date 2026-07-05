//! The game session: lifecycle and participants.

use ferrets_simulation::session::{
    FinishPolicy, GameResult, GameSession, ai_hosting::AiHosting, player_slot::PlayerSlot,
    player_type::PlayerType,
};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "contiguous")]
fn new_panics_on_non_contiguous_slot_ids() {
    GameSession::new(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None),
            PlayerSlot::occupied(2, PlayerType::Human, None),
        ],
        AiHosting::Replicated,
        FinishPolicy::LastStanding,
    );
}

#[test]
#[should_panic(expected = "local_player")]
fn new_panics_when_local_player_is_out_of_range() {
    GameSession::new(
        3,
        vec![PlayerSlot::occupied(0, PlayerType::Human, None)],
        AiHosting::Replicated,
        FinishPolicy::LastStanding,
    );
}

#[test]
fn configure_replaces_slots_and_local_player_while_pending() {
    let mut session = GameSession::new(
        0,
        vec![PlayerSlot::occupied(0, PlayerType::Human, None)],
        AiHosting::Replicated,
        FinishPolicy::LastStanding,
    );

    session.configure(
        1,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human")),
            PlayerSlot::occupied(1, PlayerType::Human, Some("orc")),
            PlayerSlot::free(2),
        ],
        AiHosting::HostOnly,
    );

    assert_eq!(session.slots().len(), 3);
    assert_eq!(session.local_player(), 1);
    assert_eq!(session.slot(1).and_then(|s| s.race()), Some("orc"));
    assert_eq!(session.ai_hosting(), AiHosting::HostOnly);
    // The dropped tracking resized to the new slot count.
    assert!(!session.is_player_dropped(2));
}

#[test]
#[should_panic(expected = "after the session started")]
fn configure_panics_after_start() {
    let mut session = pending(2);
    session.start();
    session.configure(
        0,
        vec![PlayerSlot::occupied(0, PlayerType::Human, None)],
        AiHosting::Replicated,
    );
}

//
// ─── Lifecycle / state machine ──────────────────────────────────────────────────
//

#[test]
fn new_session_is_pending() {
    let session = pending(2);

    assert!(!session.is_active());
    assert!(!session.is_running());
    assert!(!session.is_blocked());

    assert_eq!(session.result(), None);

    assert_eq!(session.local_player(), 0);
}

#[test]
fn start_runs_session() {
    let session = session(2); // the helper starts it

    assert!(session.is_active());
    assert!(session.is_running());
    assert!(!session.is_blocked());
}

#[test]
fn set_blocked_toggles_between_running_and_blocked() {
    let mut session = session(2);

    session.set_blocked(true);
    assert!(session.is_active());
    assert!(!session.is_running());
    assert!(session.is_blocked());

    session.set_blocked(false);
    assert!(session.is_active());
    assert!(session.is_running());
    assert!(!session.is_blocked());
}

#[test]
fn set_blocked_does_nothing_before_start() {
    let mut session = pending(2);

    session.set_blocked(true);

    assert!(!session.is_active());
    assert!(!session.is_running());
    assert!(!session.is_blocked());
}

#[test]
fn set_blocked_does_nothing_after_finish() {
    let mut session = session(2);
    session.finish(GameResult::Draw);

    session.set_blocked(true);

    assert!(!session.is_active());
    assert!(!session.is_running());
    assert!(!session.is_blocked());
}

#[test]
fn advance_tick_increments_counter() {
    let mut session = session(2);
    assert_eq!(session.tick(), 0);

    session.advance_tick();
    session.advance_tick();

    assert_eq!(session.tick(), 2);
}

//
// ─── Finishing ──────────────────────────────────────────────────────────────────
//

#[test]
fn finish_records_result_and_deactivates() {
    let mut session = session(2);
    assert_eq!(session.result(), None);

    session.finish(GameResult::Aborted);

    assert_eq!(session.result(), Some(GameResult::Aborted));

    assert!(!session.is_active());
    assert!(!session.is_running());
    assert!(!session.is_blocked());
}

#[test]
fn finish_keeps_first_result() {
    let mut session = session(2);

    session.finish(GameResult::Victory { winner: 0 });
    // A later finish (e.g. a desync racing a victory) must not overwrite it.
    session.finish(GameResult::Aborted);

    assert_eq!(session.result(), Some(GameResult::Victory { winner: 0 }));
}

#[test]
fn finish_policy_round_trips() {
    let mut session = session(2);
    assert_eq!(session.finish_policy(), FinishPolicy::LastStanding);

    session.set_finish_policy(FinishPolicy::Endless);

    assert_eq!(session.finish_policy(), FinishPolicy::Endless);
}

//
// ─── Races ────────────────────────────────────────────────────────────────────
//

#[test]
fn slot_race_round_trips() {
    let slot = PlayerSlot::occupied(0, PlayerType::Human, Some("human"));
    assert_eq!(slot.race(), Some("human"));

    let free = PlayerSlot::free(1);
    assert_eq!(free.race(), None);
}

#[test]
fn set_race_updates_slot() {
    let mut session = session(2);
    assert_eq!(session.slot(0).unwrap().race(), None);

    session.set_race(0, "orc");
    session.set_race(1, "human");

    assert_eq!(session.slot(0).unwrap().race(), Some("orc"));
    assert_eq!(session.slot(1).unwrap().race(), Some("human"));
}

//
// ─── Player drop ──────────────────────────────────────────────────────────────
//

#[test]
fn fresh_session_has_no_dropped_players() {
    let session = session(3);

    assert!(!session.is_player_dropped(0));
    assert!(!session.is_player_dropped(1));
    assert!(!session.is_player_dropped(2));

    assert_eq!(session.dropped_players().count(), 0);
    assert_eq!(session.active_players().collect::<Vec<_>>(), vec![0, 1, 2]);
}

#[test]
fn drop_player_marks_only_that_player() {
    let mut session = session(3);

    session.drop_player(1);

    assert!(!session.is_player_dropped(0));
    assert!(session.is_player_dropped(1));
    assert!(!session.is_player_dropped(2));

    assert_eq!(session.dropped_players().collect::<Vec<_>>(), vec![1]);
    assert_eq!(session.active_players().collect::<Vec<_>>(), vec![0, 2]);
}

#[test]
fn dropped_and_active_players_partition_slots_by_id() {
    let mut session = session(4);

    // Drop out of order to prove iteration is by id, not drop order.
    session.drop_player(2);
    session.drop_player(0);

    assert_eq!(session.dropped_players().collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(session.active_players().collect::<Vec<_>>(), vec![1, 3]);
}

#[test]
fn is_player_dropped_is_false_for_unknown_player() {
    let session = session(2);

    // Out of range never panics (it just reports "not dropped").
    assert!(!session.is_player_dropped(9));
}

#[test]
#[should_panic(expected = "dropped twice")]
fn dropping_player_twice_panics() {
    let mut session = session(2);

    session.drop_player(1);
    session.drop_player(1);
}

//
// ─── AI hosting ───────────────────────────────────────────────────────────────
//

#[test]
fn ai_hosting_defaults_to_replicated_and_is_set_at_construction() {
    assert_eq!(GameSession::default().ai_hosting(), AiHosting::Replicated);
    assert_eq!(
        mixed_session(AiHosting::HostOnly).ai_hosting(),
        AiHosting::HostOnly
    );
}

#[test]
fn replicated_sources_ai_and_free_slots_on_every_node() {
    let session = mixed_session(AiHosting::Replicated);
    let slots = session.slots();

    for is_host in [true, false] {
        assert!(session.sources_locally(&slots[0], is_host), "local human");
        assert!(!session.sources_locally(&slots[1], is_host), "remote human");
        assert!(session.sources_locally(&slots[2], is_host), "ai");
        assert!(session.sources_locally(&slots[3], is_host), "free");
    }
}

#[test]
fn host_only_sources_ai_slots_on_the_host_only() {
    let session = mixed_session(AiHosting::HostOnly);
    let slots = session.slots();

    assert!(session.sources_locally(&slots[2], true), "ai on the host");
    assert!(!session.sources_locally(&slots[2], false), "ai on a client");
    // Free slots and the local human are unaffected by the mode.
    for is_host in [true, false] {
        assert!(session.sources_locally(&slots[0], is_host), "local human");
        assert!(!session.sources_locally(&slots[1], is_host), "remote human");
        assert!(session.sources_locally(&slots[3], is_host), "free");
    }
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

/// A session with a local human, a remote human, an AI, and a free slot.
fn mixed_session(ai_hosting: AiHosting) -> GameSession {
    GameSession::new(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human")),
            PlayerSlot::occupied(1, PlayerType::Human, Some("orc")),
            PlayerSlot::occupied(2, PlayerType::Ai, Some("orc")),
            PlayerSlot::free(3),
        ],
        ai_hosting,
        FinishPolicy::LastStanding,
    )
}

/// A `players`-slot pending session of human players, local slot `0`.
fn pending(players: usize) -> GameSession {
    let slots = (0..players)
        .map(|id| PlayerSlot::occupied(id as u8, PlayerType::Human, None))
        .collect();
    GameSession::new(0, slots, AiHosting::Replicated, FinishPolicy::LastStanding)
}

/// Like [`pending`] but already started (running).
fn session(players: usize) -> GameSession {
    let mut session = pending(players);
    session.start();
    session
}
