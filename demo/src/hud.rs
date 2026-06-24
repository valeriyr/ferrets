//! Minimal HUD: a resource bar and a context line for the current selection.

use bevy::prelude::*;
use ferrets_simulation::{
    components::{
        build::BuilderStaticData,
        entity_info::EntityInfoComponent,
        health::{HealthComponent, HealthStaticData},
        resource::{ResourceCarrierComponent, ResourceSourceComponent, ResourceSourceStaticData},
        train::TrainStaticData,
    },
    resources::PlayerResources,
    selection::Selection,
    session::{GameResult, GameSession},
};

#[derive(Component)]
pub struct ResourceText;

#[derive(Component)]
pub struct HelpText;

#[derive(Component)]
pub struct SelectionText;

#[derive(Component)]
pub struct GameOverText;

/// Spawns the HUD text nodes.
pub fn setup_hud(mut commands: Commands) {
    commands.spawn((
        ResourceText,
        Text::new("Gold: 0   Wood: 0"),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.95, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
    commands.spawn((
        HelpText,
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.9, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
    commands.spawn((
        SelectionText,
        Text::new(""),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.95, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(12.0),
            ..default()
        },
    ));
    // A centered banner shown only once the game ends.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            top: Val::Percent(40.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            GameOverText,
            Text::new(""),
            TextFont {
                font_size: 64.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.95, 0.7)),
        )],
    ));
}

/// Updates the resource bar from the local player's stockpile.
pub fn update_resources(
    resources: Res<PlayerResources>,
    session: Res<GameSession>,
    mut text: Query<&mut Text, With<ResourceText>>,
) {
    let player = session.local_player();
    if let Ok(mut text) = text.single_mut() {
        **text = format!(
            "Gold: {}   Wood: {}",
            resources.amount(player, "gold"),
            resources.amount(player, "wood"),
        );
    }
}

/// Updates the context line with the selection's train/build options.
pub fn update_help(
    session: Res<GameSession>,
    selection: Res<Selection>,
    producers: Query<(
        &EntityInfoComponent,
        Option<&TrainStaticData>,
        Option<&BuilderStaticData>,
    )>,
    mut text: Query<&mut Text, With<HelpText>>,
) {
    let mut message =
        String::from("LMB select | drag box-select | RMB move/harvest/attack | F1 grid | F2 spawn");

    if let Some(&id) = selection.get(session.local_player()).first()
        && let Some((_, trainer, builder)) = producers.iter().find(|(info, ..)| info.id() == id)
    {
        if let Some(trainer) = trainer {
            let opts: Vec<String> = trainer
                .trains()
                .enumerate()
                .map(|(i, name)| format!("{}) {name}", i + 1))
                .collect();
            message = format!("Train:  {}", opts.join("   "));
        }
        if let Some(builder) = builder {
            let opts: Vec<&str> = builder.builds().collect();
            message = format!("Build [B to cycle, click to place]:  {}", opts.join(", "));
        }
    }

    if let Ok(mut text) = text.single_mut() {
        **text = message;
    }
}

/// Shows details about the current selection: name, health, and resource amounts
/// for the single selected entity, or a count when several are selected.
pub fn update_selection(
    session: Res<GameSession>,
    selection: Res<Selection>,
    entities: Query<(
        &EntityInfoComponent,
        Option<&HealthComponent>,
        Option<&HealthStaticData>,
        Option<&ResourceCarrierComponent>,
        Option<&ResourceSourceComponent>,
        Option<&ResourceSourceStaticData>,
    )>,
    mut text: Query<&mut Text, With<SelectionText>>,
) {
    let selected = selection.get(session.local_player());
    let message = match selected {
        [] => String::new(),
        [id] => entities
            .iter()
            .find(|(info, ..)| info.id() == *id)
            .map(
                |(info, health, health_data, carrier, source, source_data)| {
                    let mut parts = vec![pretty_name(info.type_name())];
                    if let (Some(health), Some(health_data)) = (health, health_data) {
                        parts.push(format!(
                            "HP {}/{}",
                            health.current(),
                            health_data.max_health()
                        ));
                    }
                    if let Some(carrier) = carrier
                        && let Some(kind) = &carrier.kind
                    {
                        parts.push(format!("carrying {} {kind}", carrier.amount));
                    }
                    if let (Some(source), Some(source_data)) = (source, source_data) {
                        parts.push(format!("{} {} left", source.amount, source_data.kind()));
                    }
                    parts.join("   ")
                },
            )
            .unwrap_or_default(),
        many => format!("{} units selected", many.len()),
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message;
    }
}

/// Shows a Victory/Defeat/Draw banner once the session has finished.
pub fn update_game_over(session: Res<GameSession>, mut text: Query<&mut Text, With<GameOverText>>) {
    let message = match session.result() {
        None => "",
        Some(GameResult::Draw) => "Draw",
        Some(GameResult::Victory { winner }) if winner == session.local_player() => "Victory!",
        Some(GameResult::Victory { .. }) => "Defeat",
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message.to_string();
    }
}

/// Turns a type id like `town_hall` into a display name like `Town Hall`.
fn pretty_name(type_name: &str) -> String {
    type_name
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
