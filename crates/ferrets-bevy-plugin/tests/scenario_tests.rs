//! The scenario check: with a runtime installed it evaluates the scripted
//! objectives inside the tick, publishes progress, and ends the session on
//! victory or defeat; without one it stays out of the way, and `Scripted` also
//! stands down the built-in last-standing check. Instantiating a scenario
//! builds exactly the scene it declares.

mod utils;

use bevy::prelude::*;
use ferrets_bevy_plugin::{
    ScenarioObjectives, install_scenario_runtime, instantiate_map, instantiate_scenario,
};
use ferrets_pathfinder::{astar::Projection, nav_pos::NavPos};
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::{
    components::{entity_info::EntityInfoComponent, resource::ResourceSourceComponent},
    content::registry::ContentRegistry,
    map::Map,
    map_data::{MapData, Placement},
    resources::{PlayerResources, StartingStock},
    scenario::Scenario,
    session::{
        GameResult, GameSession, finish_policy::FinishPolicy, player_slot::PlayerSlot,
        player_type::PlayerType,
    },
    spawn,
};

//
// ─── Outcome and progress ─────────────────────────────────────────────────────
//

#[test]
fn victory_once_objectives_are_met() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Scripted);
    install_army(&mut app);

    let world = app.world_mut();
    spawn::spawn_entity(world, "barracks", utils::pos(5, 5), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(10, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(12, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(14, 10), Some(0)).unwrap();

    utils::run_ticks(&mut app, 1);

    assert_eq!(
        app.world().resource::<GameSession>().result(),
        Some(GameResult::Victory { winner: 0 }),
    );
}

#[test]
fn defeat_when_no_units_remain() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Scripted);
    install_army(&mut app);

    // Player 0 fields nothing, so the failure condition trips immediately.
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        app.world().resource::<GameSession>().result(),
        Some(GameResult::Defeat),
    );
}

#[test]
fn objectives_reflect_partial_progress() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Scripted);
    install_army(&mut app);

    let world = app.world_mut();
    spawn::spawn_entity(world, "barracks", utils::pos(5, 5), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(10, 10), Some(0)).unwrap();

    utils::run_ticks(&mut app, 1);

    // Barracks built, but only one of three soldiers: still in progress.
    assert_eq!(app.world().resource::<GameSession>().result(), None);
    let objectives = &app.world().resource::<ScenarioObjectives>().0;
    let done: Vec<(&str, bool)> = objectives
        .iter()
        .map(|objective| (objective.id.as_str(), objective.done))
        .collect();
    assert_eq!(done, vec![("barracks", true), ("soldiers", false)]);
}

//
// ─── Standing down ────────────────────────────────────────────────────────────
//

#[test]
fn no_evaluation_without_scenario_installed() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Scripted);

    // A full army, but no scenario runtime: the check must not run.
    let world = app.world_mut();
    spawn::spawn_entity(world, "barracks", utils::pos(5, 5), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(10, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(12, 10), Some(0)).unwrap();
    spawn::spawn_entity(world, "soldier", utils::pos(14, 10), Some(0)).unwrap();

    utils::run_ticks(&mut app, 3);

    assert_eq!(app.world().resource::<GameSession>().result(), None);
}

#[test]
fn scripted_policy_stands_down_last_standing() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Scripted);

    // Only player 0 has a unit. Under LastStanding this is an immediate win for 0;
    // under Scripted the built-in check stays out, and no scenario decides it.
    let world = app.world_mut();
    spawn::spawn_entity(world, "soldier", utils::pos(10, 10), Some(0)).unwrap();

    utils::run_ticks(&mut app, 2);

    assert_eq!(app.world().resource::<GameSession>().result(), None);
}

//
// ─── Scene instantiation ──────────────────────────────────────────────────────
//

#[test]
fn instantiate_builds_declared_scene() {
    let mut app = utils::orders_app();

    instantiate_scenario(app.world_mut(), &scene_scenario());

    let map = app.world().resource::<Map>();
    assert_eq!(map.name(), "mission");
    assert_eq!((map.width(), map.height()), (16, 16));
    assert_eq!(map.start_point(0), Some(NavPos::new(2, 2)));

    let mut placed: Vec<(String, Option<u32>)> = app
        .world_mut()
        .query::<(&EntityInfoComponent, Option<&ResourceSourceComponent>)>()
        .iter(app.world())
        .map(|(info, source)| (info.type_name().to_string(), source.map(|s| s.amount)))
        .collect();
    placed.sort();
    assert_eq!(
        placed,
        vec![
            ("barracks".to_string(), None),
            ("mine".to_string(), Some(500)),
        ]
    );

    assert_eq!(
        app.world().resource::<PlayerResources>().amount(0, "gold"),
        75
    );
}

#[test]
fn placement_on_occupied_cell_is_skipped() {
    let mut app = utils::orders_app();
    let mut scenario = scene_scenario();
    // Two entities declared on the same cell: the second cannot be hosted, so
    // it is skipped — identically on every node, since all build the same data.
    scenario.map.placements.push(Placement {
        type_name: "barracks".to_string(),
        cell: (2, 2),
        owner: Some(0),
        amount: None,
    });

    instantiate_scenario(app.world_mut(), &scenario);

    let barracks = app
        .world_mut()
        .query::<&EntityInfoComponent>()
        .iter(app.world())
        .filter(|info| info.type_name() == "barracks")
        .count();
    assert_eq!(barracks, 1, "the blocked duplicate must not spawn");
}

#[test]
fn placements_of_unoccupied_slots_are_skipped() {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None),
        PlayerSlot::free(1),
    ]);
    utils::register_orders_content(&mut app);
    let mut data = scene_map();
    // A barracks declared for the free slot: nobody owns it, so it must not
    // spawn — on any node, since the slots are identical everywhere.
    data.placements.push(Placement {
        type_name: "barracks".to_string(),
        cell: (10, 10),
        owner: Some(1),
        amount: None,
    });

    instantiate_map(app.world_mut(), &data);

    let barracks = app
        .world_mut()
        .query::<&EntityInfoComponent>()
        .iter(app.world())
        .filter(|info| info.type_name() == "barracks")
        .count();
    assert_eq!(barracks, 1, "only the occupied slot's barracks spawns");
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Counts a finished barracks and three soldiers (types the orders roster
/// defines); loses when every unit is gone. Evaluated every tick.
const ARMY: &str = r#"
    define_scenario("test_army", {
        period = 1,
        objectives = {
            { id = "barracks", label = "Build a barracks" },
            { id = "soldiers", label = "Train 3 soldiers" },
        },
        check = function(state, view)
            local barracks, soldiers = 0, 0
            for _, entity in ipairs(view.my_entities) do
                if entity.type_name == "barracks" and not entity.under_construction then
                    barracks = barracks + 1
                elseif entity.type_name == "soldier" then
                    soldiers = soldiers + 1
                end
            end
            local built, trained = barracks >= 1, soldiers >= 3
            local outcome = "ongoing"
            if built and trained then
                outcome = "victory"
            elseif #view.my_entities == 0 then
                outcome = "defeat"
            end
            return {
                objectives = { barracks = built, soldiers = trained },
                outcome = outcome,
            }
        end,
    })
"#;

/// Installs the [`ARMY`] scenario runtime, judging player 0.
fn install_army(app: &mut App) {
    let content = ContentView::from_registry(app.world().resource::<ContentRegistry>());
    let runtime = LuaEngine
        .load_scenario(ARMY, &content)
        .expect("load scenario");
    install_scenario_runtime(app.world_mut(), runtime, 0);
}

/// A map placing types the orders roster defines: an owned base and a neutral
/// resource source with an overridden amount.
fn scene_map() -> MapData {
    MapData {
        name: "mission".to_string(),
        projection: Projection::Isometric,
        width: 16,
        height: 16,
        layers: vec![utils::GROUND],
        start_points: vec![(2, 2)],
        placements: vec![
            Placement {
                type_name: "barracks".to_string(),
                cell: (2, 2),
                owner: Some(0),
                amount: None,
            },
            Placement {
                type_name: "mine".to_string(),
                cell: (8, 8),
                owner: None,
                amount: Some(500),
            },
        ],
    }
}

/// A scenario on the [`scene_map`] with a starting stockpile.
fn scene_scenario() -> Scenario {
    Scenario {
        name: "mission".to_string(),
        slots: vec![PlayerSlot::occupied(0, PlayerType::Human, None)],
        judged_player: 0,
        map: scene_map(),
        stockpile: vec![StartingStock {
            player: 0,
            resource: "gold".to_string(),
            amount: 75,
        }],
        script: String::new(),
    }
}
