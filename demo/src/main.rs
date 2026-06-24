//! ferrets demo — a small 2D RTS that drives the `ferrets-simulation` engine so
//! its mechanics can be seen and manually tested.

// Bevy system signatures use large query/filter tuples and many params by design.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use bevy::prelude::*;
use ferrets_bevy::SimulationPlugin;
use ferrets_simulation::session::{
    FinishPolicy, GameSession, player_slot::PlayerSlot, player_type::PlayerType,
};

use crate::states::GameState;

mod camera;
mod content;
mod debug;
mod hud;
mod input;
mod map;
mod menu;
mod render;
mod setup;
mod states;
mod time;

fn main() {
    // Fixed 2-slot session: player 0 = local human, player 1 = passive enemy.
    // The match ends when one side has no entities left.
    let session = GameSession::new(
        0,
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, None),
            PlayerSlot::occupied(1, PlayerType::Ai, None),
        ],
        FinishPolicy::LastStanding,
    );

    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(SimulationPlugin::new(session, map::build()))
        .insert_resource(Time::<Fixed>::from_hz(20.0))
        .insert_resource(ClearColor(Color::srgb(0.18, 0.32, 0.16)))
        .init_state::<GameState>()
        .init_resource::<states::ChosenRace>()
        .init_resource::<time::TickTimer>()
        .init_resource::<input::DragStart>()
        .init_resource::<input::InputMode>()
        .init_resource::<debug::DebugState>()
        // Camera and content exist for every screen; the game scene is set up on
        // entering InGame once a race is chosen.
        .add_systems(Startup, (camera::spawn_camera, content::register_all))
        .add_systems(OnEnter(GameState::Menu), menu::setup_menu)
        .add_systems(OnExit(GameState::Menu), menu::teardown_menu)
        .add_systems(
            OnEnter(GameState::InGame),
            (hud::setup_hud, debug::setup_debug, setup::spawn_demo_scene).chain(),
        )
        // Tick-synced time: bracket each fixed step to measure and scale it, and
        // snapshot positions before the simulation advances (for interpolation).
        // Only meaningful while a game is running, so idle in the menu.
        .add_systems(
            FixedFirst,
            (time::mark_tick_start, setup::supply_ai_input).run_if(in_state(GameState::InGame)),
        )
        .add_systems(
            FixedPreUpdate,
            render::record_prev.run_if(in_state(GameState::InGame)),
        )
        .add_systems(
            FixedLast,
            time::scale_time_to_ticks.run_if(in_state(GameState::InGame)),
        )
        .add_systems(Update, menu::menu_buttons.run_if(in_state(GameState::Menu)))
        .add_systems(
            Update,
            (
                camera::pan_zoom,
                input::selection_input,
                input::order_input,
                input::train_input,
                input::build_input,
                input::placement_input,
                hud::update_resources,
                hud::update_help,
                hud::update_selection,
                hud::update_game_over,
                debug::toggle_debug,
                debug::spawn_debug,
                debug::debug_readout,
                render::draw_grid,
                (
                    render::attach_sprites,
                    render::interpolate_sprites,
                    render::draw_selection,
                    render::draw_facing,
                )
                    .chain(),
            )
                .run_if(in_state(GameState::InGame)),
        )
        .run();
}
