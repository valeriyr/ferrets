//! Race-select menu: pick a race, then enter the game.

use bevy::prelude::*;

use crate::states::{ChosenRace, GameState};

const NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);
const PRESSED: Color = Color::srgb(0.35, 0.55, 0.35);

/// Root of the menu UI, despawned when the menu closes.
#[derive(Component)]
pub struct MenuRoot;

/// Tags a button with the race id it selects.
#[derive(Component)]
pub struct RaceButton(&'static str);

/// Builds the race-select menu.
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
                Text::new("Choose your race"),
                TextFont {
                    font_size: 40.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 0.9)),
            ),
            race_button("Humans", "human"),
            race_button("Orcs", "orc"),
        ],
    ));
}

fn race_button(label: &str, race: &'static str) -> impl Bundle {
    (
        RaceButton(race),
        Button,
        Node {
            width: px(220),
            height: px(60),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(NORMAL),
        children![(
            Text::new(label),
            TextFont {
                font_size: 26.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.95)),
        )],
    )
}

/// Highlights buttons on hover and, on click, records the chosen race and
/// starts the game.
pub fn menu_buttons(
    mut buttons: Query<(&Interaction, &RaceButton, &mut BackgroundColor), Changed<Interaction>>,
    mut chosen: ResMut<ChosenRace>,
    mut next: ResMut<NextState<GameState>>,
) {
    for (interaction, race, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                *color = BackgroundColor(PRESSED);
                chosen.0 = race.0.to_string();
                next.set(GameState::InGame);
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
