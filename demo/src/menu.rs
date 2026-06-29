//! Main menu: choose a local game, host a network game, or join one.

use bevy::prelude::*;

use crate::states::{GameState, LobbyMode};

const NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);
const PRESSED: Color = Color::srgb(0.35, 0.55, 0.35);

/// Root of the menu UI, despawned when the menu closes.
#[derive(Component)]
pub struct MenuRoot;

/// Tags a button with the lobby it opens.
#[derive(Component, Clone, Copy)]
pub struct MenuButton(LobbyMode);

/// Builds the main menu.
pub fn setup_menu(mut commands: Commands) {
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
            menu_button("Local Game", LobbyMode::Local),
            menu_button("Create Network Game", LobbyMode::Host),
            menu_button("Connect To Network Game", LobbyMode::Client),
        ],
    ));
}

fn menu_button(label: &str, mode: LobbyMode) -> impl Bundle {
    (
        MenuButton(mode),
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
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                *color = BackgroundColor(PRESSED);
                commands.insert_resource(button.0);
                next.set(GameState::Lobby);
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
