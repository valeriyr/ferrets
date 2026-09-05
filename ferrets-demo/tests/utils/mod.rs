#![allow(dead_code)]

use bevy::{app::FixedMain, ecs::system::RunSystemOnce, prelude::*};
use ferrets_bevy_plugin::{PendingInput, SimulationPlugin};
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::{content::CONTENT, minimap, render, scenario, view};
use ferrets_geometry::projection::Projection;

use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::location::LocationComponent,
    events::SpawnCause,
    input::{InputFrames, PlayerFrame},
    map::Map,
    movement_model::MovementModel,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, local_role::LocalRole, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn::{self, FieldReach},
};

/// Creates an entity of `type_name` at `position` for `owner`, its field
/// sources at their initial reach, announcing nothing.
pub fn create_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
) -> Option<(Entity, SimulationId)> {
    spawn::create_entity(world, type_name, position, owner, FieldReach::Initial)
}

/// Like [`create_entity`], announcing the spawn with `cause`.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    cause: SpawnCause,
) -> Option<(Entity, SimulationId)> {
    spawn::spawn_entity(
        world,
        type_name,
        position,
        owner,
        cause,
        FieldReach::Initial,
    )
}

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
            LocalRole::Player(0),
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
            LocalRole::Player(0),
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
            if Some(player) != local_player {
                world
                    .resource_mut::<InputFrames>()
                    .push_frame(PlayerFrame::idle(player, current_tick));
            }
        }
        // The whole fixed step, not just `FixedUpdate`: the closing phases are
        // where a completed tick is recorded, tallied and retired, and a suite
        // that skipped them would not exercise anything hung there.
        world.run_schedule(FixedMain);
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
pub fn position_of(world: &mut World, entity: Entity) -> FixedUVec2 {
    world.get::<LocationComponent>(entity).unwrap().position
}

/// A position written as decimals rather than floats, so the position is the one
/// the digits name.
pub fn part_way(x: &str, y: &str) -> FixedUVec2 {
    FixedUVec2::new(cells(x), cells(y))
}

/// A length in cells, written as decimal digits rather than a float.
pub fn cells(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a length in cells"))
}

/// The same, where the value can point backwards.
pub fn signed_cells(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is an offset in cells"))
}

/// A position pinned to the bit — captured from a probe run and asserted
/// exactly ever after: any drift is a lockstep desync.
pub fn position_bits(x: u64, y: u64) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_bits(x), FixedU64::from_bits(y))
}

/// A headless demo-map game carrying the resources the view layer reads, so the
/// drawing and minimap systems can be driven directly without a window or a
/// renderer.
pub fn view_app() -> App {
    let mut app = demo_map_app(MovementModel::Continuous);
    let world = app.world_mut();
    world.insert_resource(Assets::<Image>::default());
    // The stores the sprite attachment draws from and the clocks the drawing
    // reads — the frame's own for turning, the fixed step's for interpolating —
    // which the headless app has no renderer to install.
    world.insert_resource(Assets::<Mesh>::default());
    world.insert_resource(Assets::<ColorMaterial>::default());
    world.init_resource::<Time>();
    world.init_resource::<Time<Fixed>>();
    world.init_resource::<render::FogReveal>();
    world.init_resource::<render::ObserverPerspective>();
    world.init_resource::<render::Smoothing>();
    world.init_resource::<render::Ghosts>();
    app
}

/// Builds the minimap and composes it once, as entering a game and drawing a
/// frame would.
pub fn compose_minimap(app: &mut App) {
    let world = app.world_mut();
    world
        .run_system_once(minimap::spawn_minimap)
        .expect("minimap spawns");
    world
        .run_system_once(minimap::refresh_minimap)
        .expect("minimap composes");
}

/// Points the minimap widget at a look, as switching the view setting would.
pub fn point_minimap(app: &mut App, diamond: bool) {
    app.world_mut().insert_resource(view::WorldView { diamond });
    app.world_mut()
        .run_system_once(minimap::follow_view)
        .expect("minimap follows the view");
}
