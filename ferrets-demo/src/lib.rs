//! ferrets demo — a small 2D RTS that drives the `ferrets-simulation` engine so
//! its mechanics can be seen and manually tested.

// Bevy system signatures use large query/filter tuples and many params by design.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::AiPlugin;
use ferrets_bevy_plugin::{NetworkPlugin, ReplayPlayback, ReplayPlugin, SimulationPlugin};
use ferrets_simulation::session::GameSession;

use crate::states::GameState;

pub mod ai;
mod camera;
pub mod content;
mod debug;
mod hud;
mod input;
pub mod lobby;
pub mod map;
mod menu;
mod render;
mod replay;
pub mod scenario;
pub mod setup;
pub mod skirmish;
mod states;
mod time;

/// Builds the demo app and runs it until the window closes.
pub fn run() {
    // The session starts empty and pending; the lobby configures it (slots, races,
    // local player) and starts it when the game begins.
    let session = GameSession::pending();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(SimulationPlugin::new(session, map::build()))
        .add_plugins(NetworkPlugin)
        .add_plugins(ReplayPlugin)
        .add_plugins(AiPlugin)
        .init_state::<GameState>()
        .insert_resource(Time::<Fixed>::from_hz(20.0))
        // The void outside the playable field; the field itself is drawn as
        // terrain tiles.
        .insert_resource(ClearColor(Color::srgb(0.09, 0.09, 0.11)))
        .init_resource::<time::TickTimer>()
        .init_resource::<input::DragStart>()
        .init_resource::<input::InputMode>()
        .init_resource::<input::Primary>()
        .init_resource::<input::LastClick>()
        .init_resource::<input::LastRecall>()
        .init_resource::<render::Ghosts>()
        .init_resource::<render::FogReveal>()
        .init_resource::<render::SkillPulses>()
        .init_resource::<debug::DebugState>()
        // Camera and content exist for every screen; the game scene is set up on
        // entering InGame once the lobby has configured the session.
        .add_systems(Startup, (camera::spawn_camera, content::register_all))
        // Main menu.
        .add_systems(OnEnter(GameState::Menu), menu::setup_menu)
        .add_systems(OnExit(GameState::Menu), menu::teardown_menu)
        .add_systems(
            Update,
            (
                menu::menu_buttons,
                scenario::start_scenario,
                replay::start_watching,
            )
                .chain()
                .run_if(in_state(GameState::Menu)),
        )
        // Lobby.
        .add_systems(
            OnEnter(GameState::Lobby),
            (lobby::enter_lobby, lobby::setup_lobby).chain(),
        )
        .add_systems(OnExit(GameState::Lobby), lobby::exit_lobby)
        .add_systems(
            Update,
            (
                lobby::auto_connect_client,
                lobby::host_rebind,
                lobby::poll_lobby_link,
                lobby::lobby_field_input,
                lobby::lobby_buttons,
                lobby::update_lobby_view,
                lobby::start_game,
            )
                .chain()
                .run_if(in_state(GameState::Lobby)),
        )
        // Game scene.
        .add_systems(
            OnEnter(GameState::InGame),
            (
                hud::setup_hud,
                debug::setup_debug,
                // Exactly one spawner runs: the scenario one when a scenario is
                // loaded, the symmetric demo scene otherwise. The camera frames
                // afterwards, off whichever map the spawner installed.
                setup::spawn_demo_scene.run_if(not(resource_exists::<scenario::CurrentScenario>)),
                scenario::spawn_scenario_scene.run_if(resource_exists::<scenario::CurrentScenario>),
                render::spawn_terrain_tiles,
                camera::frame_local_player,
                replay::start_recording,
            )
                .chain(),
        )
        .add_systems(OnExit(GameState::InGame), replay::teardown_session)
        // Tick-synced time: bracket each fixed step to measure and scale it, and
        // snapshot positions before the simulation advances (for interpolation).
        .add_systems(
            FixedFirst,
            time::mark_tick_start.run_if(in_state(GameState::InGame)),
        )
        .add_systems(
            FixedPreUpdate,
            render::record_prev.run_if(in_state(GameState::InGame)),
        )
        .add_systems(
            FixedLast,
            time::scale_time_to_ticks.run_if(in_state(GameState::InGame)),
        )
        // Command-producing input only when a live player is at the controls; during
        // replay playback the recorded frames are the sole input, so stray clicks
        // must not enter the queue.
        //
        // Chained: the mode-consuming systems (targeting, placement) run last,
        // so when they handle a click and reset the mode to Normal, the
        // mode-gated systems have already seen the armed mode this frame — an
        // unordered mode flip would let the same click also select or order.
        .add_systems(
            Update,
            (
                input::track_primary,
                input::pause_input,
                input::selection_input,
                input::order_input,
                input::stance_input,
                input::control_group_input,
                // HUD button clicks emit commands / set placement mode, so they
                // belong to the live-input phase and are silenced during replay
                // (unlike the display-only HUD systems in the viewing group).
                hud::command_card_input,
                hud::build_card_input,
                hud::skill_card_input,
                hud::group_roster_input,
                input::order_mode_input,
                input::targeting_input,
                input::placement_input,
                // F2 sandbox spawn issues a Spawn command, so it counts as input too.
                debug::spawn_debug,
            )
                .chain()
                .run_if(in_state(GameState::InGame).and(not(resource_exists::<ReplayPlayback>))),
        )
        // Viewing, HUD, debug, and rendering run for both live games and playback.
        .add_systems(
            Update,
            (
                camera::pan_zoom,
                render::toggle_fog_reveal,
                hud::update_resources,
                hud::update_help,
                hud::update_command_card,
                hud::update_group_roster,
                hud::update_selection,
                hud::update_objectives,
                hud::update_game_over,
                hud::update_replay_note,
                hud::leave_button,
                debug::toggle_debug,
                debug::debug_readout,
                debug::draw_grid,
                // Reads the fog Visibility that interpolate_sprites writes, so it
                // must run after it — otherwise a fogged unit's orders can flash.
                debug::draw_orders.after(render::interpolate_sprites),
                (
                    render::attach_sprites,
                    render::interpolate_sprites,
                    render::update_fog_overlay,
                    render::draw_ghosts,
                    render::draw_selection,
                    render::draw_skill_pulses,
                    render::draw_facing,
                    render::draw_rally,
                )
                    .chain(),
            )
                .run_if(in_state(GameState::InGame)),
        );

    app.run();
}
