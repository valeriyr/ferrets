//! Composing session slots from a map's seats and a scenario's cast.

use ferrets_geometry::projection::Projection;
use ferrets_simulation::{
    map_data::MapData,
    scenario::{Scenario, ScenarioPlayer},
    session::{
        ai_vision::AiVision,
        player_slot::{self, PlayerSlot},
        player_type::PlayerType,
    },
};

//
// ─── Vacant slots ─────────────────────────────────────────────────────────────
//

#[test]
fn vacant_slots_mirror_map_seats_by_id() {
    let slots = player_slot::vacant_slots(&scene_map(), AiVision::Filtered);

    assert_eq!(
        slots,
        vec![
            PlayerSlot::free(0),
            PlayerSlot::environment(1, AiVision::Filtered),
            PlayerSlot::free(2),
        ],
    );
}

//
// ─── Scenario slots ───────────────────────────────────────────────────────────
//

#[test]
fn scenario_slots_seat_cast_and_leave_uncast_seats_vacant() {
    let scenario = mission(vec![ScenarioPlayer {
        seat: 2,
        player_type: PlayerType::Ai {
            vision: AiVision::Filtered,
        },
        race: Some("orc".to_string()),
        team: Some(1),
    }]);

    let slots = player_slot::scenario_slots(&scenario, AiVision::Filtered);

    assert_eq!(
        slots,
        vec![
            PlayerSlot::free(0),
            PlayerSlot::environment(1, AiVision::Filtered),
            PlayerSlot::occupied(
                2,
                PlayerType::Ai {
                    vision: AiVision::Filtered
                },
                Some("orc"),
                Some(1)
            ),
        ],
    );
}

#[test]
#[should_panic(expected = "scenario 'mission' casts seat 5, which the map does not declare")]
fn scenario_casting_undeclared_seat_panics() {
    let scenario = mission(vec![cast(5)]);

    player_slot::scenario_slots(&scenario, AiVision::Filtered);
}

#[test]
#[should_panic(expected = "scenario 'mission' casts seat 0 twice, or casts an environment seat")]
fn scenario_casting_seat_twice_panics() {
    let scenario = mission(vec![cast(0), cast(0)]);

    player_slot::scenario_slots(&scenario, AiVision::Filtered);
}

#[test]
#[should_panic(expected = "scenario 'mission' casts seat 1 twice, or casts an environment seat")]
fn scenario_casting_environment_seat_panics() {
    let scenario = mission(vec![cast(1)]);

    player_slot::scenario_slots(&scenario, AiVision::Filtered);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A map declaring a player seat, an environment seat, and another player
/// seat, in that id order.
fn scene_map() -> MapData {
    let mut map = MapData::new("scene", Projection::Isometric, 8, 8);
    map.add_player_slot((1, 1));
    map.add_environment_slot();
    map.add_player_slot((6, 6));
    map
}

/// A scenario named "mission" on [`scene_map`] with the given cast.
fn mission(players: Vec<ScenarioPlayer>) -> Scenario {
    Scenario {
        name: "mission".to_string(),
        players,
        judged_player: 0,
        map: scene_map(),
        stockpile: vec![],
        script: String::new(),
    }
}

/// A human cast entry for `seat`, without race or team.
fn cast(seat: u8) -> ScenarioPlayer {
    ScenarioPlayer {
        seat,
        player_type: PlayerType::Human,
        race: None,
        team: None,
    }
}
