#![allow(dead_code)]

use bevy::prelude::*;
use ferrets_bevy_plugin::{PendingInput, SimulationPlugin};
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::{content::CONTENT, scenario};
use ferrets_geometry::projection::Projection;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    input::{InputFrames, PlayerFrame},
    map::Map,
    movement_model::MovementModel,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, player_slot::PlayerSlot, player_type::PlayerType,
    },
};

/// A headless single-player game on the built-in mission's map and content,
/// forced onto the given movement model.
pub fn scenario_app(model: MovementModel) -> App {
    let slots = vec![PlayerSlot::occupied(
        0,
        PlayerType::Human,
        Some("human"),
        None,
    )];
    let mission = scenario::builtin_mission(Projection::Isometric, model);
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let game_map = Map::from_data(&mission.map, &registry);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            "build_army",
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        game_map,
    ));
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() = registry;
        // The demo's own scenario spawner, so a test plays the scene a player
        // would (its map, its stockpile, its player stats).
        world.insert_resource(scenario::CurrentScenario(mission));
        scenario::spawn_scenario_scene(world);
    }
    app
}

/// A headless game on the **demo skirmish map** — the 96×96 field with its
/// central lake and the rivers that split it — and no placements at all, so a
/// test sees only the terrain and whatever it spawns itself.
///
/// Two seats are occupied and on no team, so slots `0` and `1` are hostile to
/// each other and a test can stage a fight by choosing owners. The built-in
/// mission's map is 32×32 and all grass, so anything about water, rivers, fords
/// or two sides has to be tested here instead.
pub fn demo_map_app(model: MovementModel) -> App {
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(1, PlayerType::Human, Some("orc"), None),
    ];
    let mut data = ferrets_demo::map::data();
    data.set_movement_model(model);
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content");
    let game_map = Map::from_data(&data, &registry);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            0,
            slots,
            ferrets_demo::map::NAME,
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        game_map,
    ));
    {
        let world = app.world_mut();
        *world.resource_mut::<ContentRegistry>() = registry;
        world.resource_mut::<GameSession>().start();
    }
    app
}

/// The cell a position falls in, as a `(x, y)` pair for terse assertions.
pub fn cell_of(position: FixedUVec2) -> (u32, u32) {
    (
        position.x.floor().to_num::<u32>(),
        position.y.floor().to_num::<u32>(),
    )
}

/// A position at a cell's origin corner, which is where a unit at rest stands.
pub fn at_cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// Advances the app by `ticks` fixed steps, feeding an idle frame for every
/// player the local node does not source itself, so the lockstep loop never
/// blocks waiting on absent peers.
pub fn run_ticks(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        let world = app.world_mut();
        let (current_tick, local_player, players) = {
            let session = world.resource::<GameSession>();
            let players: Vec<_> = session.slots().iter().map(|slot| slot.id()).collect();
            (session.tick(), session.local_player(), players)
        };
        for player in players {
            if player != local_player {
                world
                    .resource_mut::<InputFrames>()
                    .push_frame(PlayerFrame::idle(player, current_tick));
            }
        }
        world.run_schedule(FixedUpdate);
    }
}

/// Ticks needed for a queued command to reach the simulation (the input sync
/// latency the plugin schedules commands at).
pub const APPLY: u32 = 3;

/// Queues a command from the local player, applied after [`APPLY`] ticks.
pub fn push_command(app: &mut App, command: PlayerCommand) {
    app.world_mut().resource_mut::<PendingInput>().push(command);
}

/// Selects `id` for the local player with the given mode.
pub fn select(
    app: &mut App,
    id: ferrets_simulation::simulation_id::SimulationId,
    mode: SelectMode,
) {
    push_command(app, PlayerCommand::SelectById { id, mode });
}

/// The entity's continuous position — sub-cell precise.
pub fn position_of(world: &mut bevy::prelude::World, entity: bevy::prelude::Entity) -> FixedUVec2 {
    world
        .get::<ferrets_simulation::components::location::LocationComponent>(entity)
        .unwrap()
        .position
}

/// A position pinned to the bit — captured from a probe run and asserted
/// exactly ever after: any drift is a lockstep desync.
pub fn position_bits(x: u64, y: u64) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_bits(x), FixedU64::from_bits(y))
}
