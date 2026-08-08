//! Main menu: choose a local game, host a network game, or join one.

use bevy::prelude::*;
use ferrets_simulation::movement_model::MovementModel;

use crate::{
    replay::WatchReplayRequested,
    scenario::ScenarioRequested,
    settings::{Settings, View},
    states::{GameState, LobbyMode},
};

const NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);
const PRESSED: Color = Color::srgb(0.35, 0.55, 0.35);

/// Root of the menu UI, despawned when the menu closes.
#[derive(Component)]
pub struct MenuRoot;

/// A pre-game option cycled from the menu, doubling as the marker on its
/// button's label so a click can rewrite the text it changed.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Cycles [`Settings::movement_model`].
    Movement,
    /// Cycles [`Settings::view`].
    View,
}

/// Tags a button with the action it triggers.
#[derive(Component, Clone, Copy)]
pub enum MenuButton {
    /// Opens the lobby in the given mode.
    Lobby(LobbyMode),
    /// Starts the scripted story mission.
    Scenario,
    /// Opens a replay file to watch.
    WatchReplay,
    /// Cycles a pre-game option.
    Cycle(Setting),
}

/// A settings button's label under the current choices.
fn setting_label(setting: Setting, settings: &Settings) -> String {
    match setting {
        Setting::Movement => {
            let name = match settings.movement_model {
                MovementModel::Cell => "Cell",
                MovementModel::Continuous => "Continuous",
            };
            format!("Movement: {name}")
        }
        Setting::View => {
            let name = match settings.view {
                View::IsometricDiamond => "Isometric Diamond",
                View::IsometricSquare => "Isometric Square",
                View::Orthogonal => "Orthogonal",
            };
            format!("World: {name}")
        }
    }
}

/// Advances one option to its next value.
fn cycle_setting(setting: Setting, settings: &mut Settings) {
    match setting {
        Setting::Movement => {
            settings.movement_model = match settings.movement_model {
                MovementModel::Cell => MovementModel::Continuous,
                MovementModel::Continuous => MovementModel::Cell,
            };
        }
        Setting::View => {
            settings.view = match settings.view {
                View::IsometricDiamond => View::IsometricSquare,
                View::IsometricSquare => View::Orthogonal,
                View::Orthogonal => View::IsometricDiamond,
            };
        }
    }
}

/// Builds the main menu.
pub fn setup_menu(mut commands: Commands, settings: Res<Settings>) {
    commands.spawn((
        MenuRoot,
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(20),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
        children![
            (
                Text::new("Ferrets Demo"),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 0.9)),
            ),
            menu_button("Local Game", MenuButton::Lobby(LobbyMode::Local)),
            menu_button("Create Network Game", MenuButton::Lobby(LobbyMode::Host)),
            menu_button(
                "Connect To Network Game",
                MenuButton::Lobby(LobbyMode::Client)
            ),
            menu_button("Scenario", MenuButton::Scenario),
            menu_button("Watch Replay", MenuButton::WatchReplay),
            // A gap separates what starts a game from what configures it.
            (Node {
                height: px(28),
                ..default()
            },),
            setting_button(Setting::Movement, &settings),
            setting_button(Setting::View, &settings),
        ],
    ));
}

/// A settings button: cycles its option on click, label naming the current
/// value.
fn setting_button(setting: Setting, settings: &Settings) -> impl Bundle {
    (
        MenuButton::Cycle(setting),
        Button,
        Node {
            width: px(380),
            height: px(56),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(NORMAL),
        children![(
            setting,
            Text::new(setting_label(setting, settings)),
            TextFont {
                font_size: 24.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.95)),
        )],
    )
}

fn menu_button(label: &str, button: MenuButton) -> impl Bundle {
    (
        button,
        Button,
        Node {
            width: px(380),
            height: px(56),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(NORMAL),
        children![(
            Text::new(label),
            TextFont {
                font_size: 24.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.95)),
        )],
    )
}

/// Highlights buttons and, on click, records the lobby mode and opens the lobby.
pub fn menu_buttons(
    mut buttons: Query<(&Interaction, &MenuButton, &mut BackgroundColor), Changed<Interaction>>,
    mut commands: Commands,
    mut next: ResMut<NextState<GameState>>,
    mut settings: ResMut<Settings>,
    mut labels: Query<(&mut Text, &Setting)>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                *color = BackgroundColor(PRESSED);
                match button {
                    MenuButton::Lobby(mode) => {
                        commands.insert_resource(*mode);
                        next.set(GameState::Lobby);
                    }
                    // The session is configured and entered by start_scenario, which
                    // runs in the menu; staying here keeps the menu responsive if the
                    // scenario fails to load.
                    MenuButton::Scenario => commands.insert_resource(ScenarioRequested),
                    // The dialog is opened (and the game entered) by start_watching,
                    // which runs in the menu; staying here keeps the menu responsive
                    // if the dialog is cancelled.
                    MenuButton::WatchReplay => commands.insert_resource(WatchReplayRequested),
                    MenuButton::Cycle(setting) => {
                        cycle_setting(*setting, &mut settings);
                        for (mut label, &of) in &mut labels {
                            if of == *setting {
                                *label = Text::new(setting_label(of, &settings));
                            }
                        }
                    }
                }
            }
            Interaction::Hovered => *color = BackgroundColor(HOVERED),
            Interaction::None => *color = BackgroundColor(NORMAL),
        }
    }
}

/// Removes the menu UI when leaving the menu.
pub fn teardown_menu(mut commands: Commands, roots: Query<Entity, With<MenuRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}
