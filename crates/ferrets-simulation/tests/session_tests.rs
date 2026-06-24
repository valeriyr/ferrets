//! Player slots: race assignment and lookup.

use ferrets_simulation::session::{
    FinishPolicy, GameSession, player_slot::PlayerSlot, player_type::PlayerType,
};

#[test]
fn slot_race_round_trips() {
    let slot = PlayerSlot::occupied(0, PlayerType::Human, Some("human"));
    assert_eq!(slot.race(), Some("human"));

    let free = PlayerSlot::free(1);
    assert_eq!(free.race(), None);
}

#[test]
fn set_race_updates_the_slot() {
    let mut session = GameSession::new(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None),
            PlayerSlot::occupied(1, PlayerType::Ai, None),
        ],
        FinishPolicy::LastStanding,
    );
    assert_eq!(session.slot(0).unwrap().race(), None);

    session.set_race(0, "orc");
    session.set_race(1, "human");

    assert_eq!(session.slot(0).unwrap().race(), Some("orc"));
    assert_eq!(session.slot(1).unwrap().race(), Some("human"));
}
