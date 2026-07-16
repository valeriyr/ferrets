//! The game session: lifecycle and participants.

use ferrets_simulation::session::drop_policy::DropPolicy;
use ferrets_simulation::session::finish_policy::FinishPolicy;
use ferrets_simulation::session::{
    GameResult, GameSession, Winner, ai_hosting::AiHosting, authority::Authority,
    player_slot::PlayerSlot, player_type::PlayerType,
};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "contiguous")]
fn configured_panics_on_non_contiguous_slot_ids() {
    configured(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::occupied(2, PlayerType::Human, None, None),
        ],
    );
}

#[test]
#[should_panic(expected = "local_player")]
fn configured_panics_when_local_player_is_out_of_range() {
    configured(3, humans(1));
}

#[test]
fn configure_replaces_slots_and_local_player_while_pending() {
    let mut session = configured(0, humans(1));

    session.configure(
        1,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
            PlayerSlot::occupied(1, PlayerType::Human, Some("orc"), None),
            PlayerSlot::free(2),
        ],
        "test",
        Authority::Host {
            ai_hosting: AiHosting::Host,
        },
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
    );

    assert_eq!(session.slots().len(), 3);
    assert_eq!(session.local_player(), 1);
    assert_eq!(session.slot(1).and_then(|s| s.race()), Some("orc"));
    assert_eq!(session.ai_hosting(), AiHosting::Host);
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
        vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)],
        "test",
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
    );
}

//
// ─── Lifecycle / state machine ──────────────────────────────────────────────────
//

#[test]
fn configured_session_starts_pending() {
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

    session.finish(GameResult::Victory {
        winner: Winner::Player(0),
    });
    // A later finish (e.g. a desync racing a victory) must not overwrite it.
    session.finish(GameResult::Aborted);

    assert_eq!(
        session.result(),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
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
    let slot = PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None);
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
// ─── Teams ──────────────────────────────────────────────────────────────────────
//

#[test]
fn slot_team_round_trips() {
    let slot = PlayerSlot::occupied(0, PlayerType::Human, None, Some(2));
    assert_eq!(slot.team(), Some(2));

    // A slot starts on no team, whether occupied or free.
    assert_eq!(
        PlayerSlot::occupied(1, PlayerType::Ai, None, None).team(),
        None
    );
    assert_eq!(PlayerSlot::free(2).team(), None);
}

#[test]
fn set_team_updates_slot() {
    let mut slot = PlayerSlot::occupied(0, PlayerType::Human, None, None);
    slot.set_team(Some(3));
    assert_eq!(slot.team(), Some(3));

    slot.set_team(None);
    assert_eq!(slot.team(), None);
}

#[test]
fn same_team_allies_players_different_teams_do_not() {
    let session = teams(&[Some(1), Some(1), Some(2)]);
    // Same team → allied, symmetrically.
    assert!(session.are_allied(0, 1));
    assert!(session.are_allied(1, 0));
    // Different teams → not allied.
    assert!(!session.are_allied(0, 2));
}

#[test]
fn player_with_no_team_is_allied_with_no_one_but_itself() {
    let session = teams(&[None, None, Some(1)]);
    // Teamless players are hostile to everyone, including other teamless ones.
    assert!(!session.are_allied(0, 1));
    assert!(!session.are_allied(0, 2));
    // A player is always allied with itself.
    assert!(session.are_allied(0, 0));
}

#[test]
fn is_winner_covers_the_whole_team_or_the_lone_player() {
    let session = teams(&[Some(1), Some(1), None]);
    // A team victory: every member of that team shares it, no one else.
    assert!(session.is_winner(0, Winner::Team(1)));
    assert!(session.is_winner(1, Winner::Team(1)));
    assert!(!session.is_winner(2, Winner::Team(1)));
    // A lone-player victory: only that player.
    assert!(session.is_winner(2, Winner::Player(2)));
    assert!(!session.is_winner(0, Winner::Player(2)));
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

    session.drop_player(1, 0);

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
    session.drop_player(2, 0);
    session.drop_player(0, 0);

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

    session.drop_player(1, 0);
    session.drop_player(1, 1);
}

#[test]
fn required_players_stops_including_player_at_its_drop_tick() {
    let mut session = session(3);

    session.drop_player(1, 5);

    // Ticks the player was live for keep requiring its input; from the drop
    // tick on it no longer counts.
    assert_eq!(session.required_players(4), vec![0, 1, 2]);
    assert_eq!(session.required_players(5), vec![0, 2]);
    assert_eq!(session.required_players(9), vec![0, 2]);
}

#[test]
fn required_players_skips_free_slots() {
    let mut session = configured(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None, None),
            PlayerSlot::free(1),
            PlayerSlot::occupied(2, PlayerType::Ai, None, None),
        ],
    );
    session.start();

    assert_eq!(session.required_players(0), vec![0, 2]);
}

//
// ─── AI hosting ───────────────────────────────────────────────────────────────
//

#[test]
fn ai_hosting_is_set_at_construction() {
    assert_eq!(
        mixed_session(AiHosting::Replicated).ai_hosting(),
        AiHosting::Replicated,
    );
    assert_eq!(mixed_session(AiHosting::Host).ai_hosting(), AiHosting::Host);
}

#[test]
#[should_panic(expected = "local_player 0 is not in slots (len 0)")]
fn configured_panics_on_empty_slots() {
    // A slotless session exists only as the pending placeholder; the public
    // constructor demands a coherent slot list.
    configured(0, Vec::new());
}

#[test]
fn pending_session_is_inert_until_configured() {
    let session = GameSession::pending();

    assert!(session.slots().is_empty());
    assert!(!session.is_active());
    assert_eq!(session.tick(), 0);
}

#[test]
fn replicated_sources_ai_and_free_slots_on_every_node() {
    let session = mixed_session(AiHosting::Replicated);
    let slots = session.slots();

    for is_host in [true, false] {
        assert!(session.sources_locally(&slots[0], is_host), "local human");
        assert!(!session.sources_locally(&slots[1], is_host), "remote human");
        assert!(session.sources_locally(&slots[2], is_host), "ai");
        assert!(!session.sources_locally(&slots[3], is_host), "free");
    }
}

#[test]
fn host_only_sources_ai_slots_on_host_only() {
    let session = mixed_session(AiHosting::Host);
    let slots = session.slots();

    assert!(session.sources_locally(&slots[2], true), "ai on the host");
    assert!(!session.sources_locally(&slots[2], false), "ai on a client");
    // Free slots and the local human are unaffected by the mode.
    for is_host in [true, false] {
        assert!(session.sources_locally(&slots[0], is_host), "local human");
        assert!(!session.sources_locally(&slots[1], is_host), "remote human");
        assert!(!session.sources_locally(&slots[3], is_host), "free");
    }
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

/// A session with a local human, a remote human, an AI, and a free slot.
fn mixed_session(ai_hosting: AiHosting) -> GameSession {
    GameSession::configured(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
            PlayerSlot::occupied(1, PlayerType::Human, Some("orc"), None),
            PlayerSlot::occupied(2, PlayerType::Ai, Some("orc"), None),
            PlayerSlot::free(3),
        ],
        "test",
        Authority::Host { ai_hosting },
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
    )
}

/// A session over `slots` with the suite's baseline choices: host authority
/// with replicated AI, automatic drops, last-standing finish. Tests that vary
/// one of the choices construct explicitly instead.
fn configured(local_player: u8, slots: Vec<PlayerSlot>) -> GameSession {
    GameSession::configured(
        local_player,
        slots,
        "test",
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
    )
}

/// `count` occupied raceless human slots with contiguous ids.
fn humans(count: usize) -> Vec<PlayerSlot> {
    (0..count)
        .map(|id| PlayerSlot::occupied(id as u8, PlayerType::Human, None, None))
        .collect()
}

/// A session seating one human per entry in `assignments` (each entry that
/// player's team, `None` for no team), with the local player at slot `0`.
fn teams(assignments: &[Option<u8>]) -> GameSession {
    let slots = assignments
        .iter()
        .enumerate()
        .map(|(id, team)| PlayerSlot::occupied(id as u8, PlayerType::Human, None, *team))
        .collect();
    configured(0, slots)
}

/// A `players`-slot pending session of human players, local slot `0`.
fn pending(players: usize) -> GameSession {
    configured(0, humans(players))
}

/// Like [`pending`] but already started (running).
fn session(players: usize) -> GameSession {
    let mut session = pending(players);
    session.start();
    session
}
