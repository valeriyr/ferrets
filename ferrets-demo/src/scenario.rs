//! The demo story mission: a small map with a single human player whose
//! objectives and win/loss are decided by a Lua scenario script.
//!
//! The mission is data — the built-in [`Scenario`] from [`builtin_mission`].
//! Selecting "Scenario" in the menu inserts [`ScenarioRequested`]; the
//! menu-side [`start_scenario`] then configures the session, loads the
//! scenario runtime, and enters the game. On entering,
//! [`spawn_scenario_scene`] builds the declared scene. Cells are `(x, y)`
//! with `x` right and `y` down.

use bevy::prelude::*;
use ferrets_bevy_plugin::{install_game_resources, install_scenario_runtime, instantiate_scenario};
use ferrets_pathfinder::astar::Projection;
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::{
    content::registry::ContentRegistry,
    map_data::{MapData, Placement},
    resources::StartingStock,
    scenario::{Scenario, ScenarioPlayer},
    session::{
        GameSession,
        ai_hosting::AiHosting,
        authority::Authority,
        drop_policy::DropPolicy,
        finish_policy::FinishPolicy,
        player_slot::{self, PlayerId},
        player_type::PlayerType,
    },
};

use crate::states::GameState;

/// The scenario script. The engine holds the objective list (id + label,
/// fixing the display order); the script only reports which are met and the
/// overall outcome, evaluated on the integer view every `period` ticks inside
/// the deterministic tick loop.
///
/// Win by fielding a barracks and three archers; lose if every unit is gone.
const SCRIPT: &str = r#"
    define_scenario("build_army", {
        period = 10,
        objectives = {
            { id = "barracks", label = "Build a barracks" },
            { id = "archers", label = "Train 3 archers" },
        },
        check = function(state, view)
            local barracks, archers = 0, 0
            for _, entity in ipairs(view.my_entities) do
                if entity.type_name == "barracks" and not entity.under_construction then
                    barracks = barracks + 1
                elseif entity.type_name == "archer" then
                    archers = archers + 1
                end
            end

            local built = barracks >= 1
            local trained = archers >= 3

            local outcome = "ongoing"
            if built and trained then
                outcome = "victory"
            elseif #view.my_entities == 0 then
                -- Everything is gone; the mission is lost.
                outcome = "defeat"
            end

            return {
                objectives = { barracks = built, archers = trained },
                outcome = outcome,
            }
        end,
    })
"#;

/// The lone human player of this mission.
const PLAYER: PlayerId = 0;

/// The human's base cell; also where the camera opens.
const START: (u32, u32) = (6, 6);

/// Inserted by the menu to request the scenario; consumed by [`start_scenario`].
#[derive(Resource)]
pub struct ScenarioRequested;

/// The scenario the current game runs: the scene spawner builds from it and a
/// recording embeds it. Absent outside a scenario game.
#[derive(Resource)]
pub struct CurrentScenario(pub Scenario);

/// The built-in mission definition.
///
/// The starting stockpile is enough to build the barracks (200 gold, 100 wood)
/// and train three archers (240 gold) without harvesting, so the objective is
/// the point rather than the economy.
pub fn builtin_mission() -> Scenario {
    let mut map = MapData::new("build_army", Projection::Isometric, 32, 32);
    map.fill_terrain("grass");
    map.add_player_slot(START);

    // The mission's authored base, plus a gold mine within reach and a small
    // wood grove.
    map.add_placement(place("town_hall", START, Some(PLAYER), None));
    map.add_placement(place("peasant", (START.0 + 3, START.1), Some(PLAYER), None));
    map.add_placement(place(
        "peasant",
        (START.0 + 3, START.1 + 1),
        Some(PLAYER),
        None,
    ));
    map.add_placement(place("gold_mine", (13, 6), None, Some(5000)));
    for cell in [(5, 13), (6, 13), (5, 14), (6, 14)] {
        map.add_placement(place("tree", cell, None, Some(400)));
    }

    Scenario {
        name: "build_army".to_string(),
        players: vec![ScenarioPlayer {
            seat: PLAYER,
            player_type: PlayerType::Human,
            race: Some("human".to_string()),
            team: None,
        }],
        judged_player: PLAYER,
        map,
        stockpile: vec![
            StartingStock {
                player: PLAYER,
                resource: "gold".to_string(),
                amount: 600,
            },
            StartingStock {
                player: PLAYER,
                resource: "wood".to_string(),
                amount: 200,
            },
        ],
        script: SCRIPT.to_string(),
    }
}

/// Configures a single-player scripted session for the built-in mission, loads
/// the scenario runtime, and enters the game. Runs in the menu so a load
/// failure leaves the menu alive.
pub fn start_scenario(world: &mut World) {
    if world.remove_resource::<ScenarioRequested>().is_none() {
        return;
    }

    let scenario = builtin_mission();
    let content = ContentView::from_registry(world.resource::<ContentRegistry>());
    let runtime = match LuaEngine.load_scenario(&scenario.script, &content) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to load scenario script: {error}");
            return;
        }
    };

    {
        let mut session = world.resource_mut::<GameSession>();
        session.configure(
            scenario.judged_player,
            player_slot::scenario_slots(&scenario),
            scenario.map.name(),
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Scripted,
        );
    }
    install_game_resources(world);
    install_scenario_runtime(world, runtime, scenario.judged_player);
    world.insert_resource(CurrentScenario(scenario));

    world
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
}

/// Builds the loaded scenario's scene — map, base, resources, stockpile — and
/// starts the simulation. The scenario's script then drives the objectives and
/// outcome.
pub fn spawn_scenario_scene(world: &mut World) {
    let scenario = world.resource::<CurrentScenario>().0.clone();
    instantiate_scenario(world, &scenario);
    world.resource_mut::<GameSession>().start();
}

fn place(
    type_name: &str,
    cell: (u32, u32),
    owner: Option<PlayerId>,
    amount: Option<u32>,
) -> Placement {
    Placement {
        type_name: type_name.to_string(),
        cell,
        owner,
        amount,
    }
}
