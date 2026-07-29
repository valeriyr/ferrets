//! Minimal HUD: a resource bar and a context line for the current selection.

use bevy::prelude::*;
use ferrets_bevy_plugin::{PendingInput, ReplayPlayback, ScenarioObjectives};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        buffs::BuffsComponent,
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        owner::OwnerComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::StanceComponent,
    },
    content::registry::ContentRegistry,
    content::skills::SkillId,
    content::stats::StatId,
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    resources::PlayerResources,
    selection::Selection,
    session::{GameResult, GameSession},
};

use crate::input::{InputMode, Primary};
use crate::states::{GameState, InGameUi};

const BUTTON_NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const BUTTON_HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);
// Build buttons get a cooler tint so they stay distinct from train buttons on an
// entity that can do both.
const BUILD_NORMAL: Color = Color::srgb(0.16, 0.22, 0.30);
const BUILD_HOVERED: Color = Color::srgb(0.24, 0.32, 0.44);
// Skill buttons get a warm violet tint so abilities read apart from train/build.
const SKILL_NORMAL: Color = Color::srgb(0.26, 0.18, 0.30);
const SKILL_HOVERED: Color = Color::srgb(0.38, 0.26, 0.44);

#[derive(Component)]
pub struct ResourceText;

#[derive(Component)]
pub struct HelpText;

#[derive(Component)]
pub struct SelectionText;

#[derive(Component)]
pub struct GameOverText;

/// The scenario objectives checklist, shown only during a scripted mission.
#[derive(Component)]
pub struct ObjectivesText;

/// A line shown during replay playback (its end, or a determinism mismatch).
#[derive(Component)]
pub struct ReplayNote;

/// The button that returns to the main menu.
#[derive(Component)]
pub struct LeaveButton;

/// The command-card container; train buttons for the primary producer are its children.
#[derive(Component)]
pub struct CommandCard;

/// A command-card button that queues an entity type on the primary producer.
#[derive(Component)]
pub struct TrainButton {
    /// The entity type this button queues.
    type_name: String,
}

/// A command-card button that starts placing a building with the primary builder.
#[derive(Component)]
pub struct BuildButton {
    /// The building type this button starts placing.
    type_name: String,
}

/// A command-card button that casts a skill on the selection.
#[derive(Component)]
pub struct SkillButton {
    /// The skill this button casts.
    skill: SkillId,
}

/// The control-group roster container; a chip per non-empty group is a child.
#[derive(Component)]
pub struct GroupRoster;

/// A roster chip that recalls a control group when clicked.
#[derive(Component)]
pub struct GroupButton {
    /// The control group this chip recalls.
    group: u8,
}

/// Spawns the HUD text nodes.
pub fn setup_hud(mut commands: Commands) {
    commands.spawn((
        InGameUi,
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
        InGameUi,
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
        InGameUi,
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
    // Scenario objectives, top-left below the resource bar. Empty (and invisible)
    // outside a scripted mission.
    commands.spawn((
        InGameUi,
        ObjectivesText,
        Text::new(""),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.95, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(80.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
    // A centered banner shown only once the game ends.
    commands.spawn((
        InGameUi,
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
    // A note shown during replay playback, below the game-over banner.
    commands.spawn((
        InGameUi,
        ReplayNote,
        Text::new(""),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            top: Val::Percent(52.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
    // Control-group roster: a chip per non-empty group, stacked just above the
    // command card so all unit-control UI sits together in the bottom-left, clear
    // of the resource bar, debug readout, and objectives.
    commands.spawn((
        InGameUi,
        GroupRoster,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(66.0),
            left: Val::Px(10.0),
            column_gap: Val::Px(6.0),
            ..default()
        },
    ));
    // Command card: train/build buttons for the selected producer (bottom left).
    commands.spawn((
        InGameUi,
        CommandCard,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(34.0),
            left: Val::Px(10.0),
            column_gap: Val::Px(6.0),
            ..default()
        },
    ));
    // Returns to the main menu.
    commands.spawn((
        InGameUi,
        LeaveButton,
        Button,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            right: Val::Px(12.0),
            width: Val::Px(96.0),
            height: Val::Px(34.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(BUTTON_NORMAL),
        children![(
            Text::new("Leave"),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.95)),
        )],
    ));
}

/// Highlights the Leave button and returns to the menu when it is pressed.
pub fn leave_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<LeaveButton>),
    >,
    mut next: ResMut<NextState<GameState>>,
) {
    for (interaction, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => next.set(GameState::Menu),
            Interaction::Hovered => *color = BackgroundColor(BUTTON_HOVERED),
            Interaction::None => *color = BackgroundColor(BUTTON_NORMAL),
        }
    }
}

/// Shows replay-playback status: nothing during a live game, an "ended" note once
/// playback freezes, or a divergence warning if a recorded checksum failed.
pub fn update_replay_note(
    playback: Option<Res<ReplayPlayback>>,
    mut text: Query<&mut Text, With<ReplayNote>>,
) {
    let message = match playback {
        Some(playback) if playback.mismatch().is_some() => {
            format!("Replay diverged at tick {}", playback.mismatch().unwrap())
        }
        Some(playback) if playback.is_done() => String::from("Replay ended"),
        _ => String::new(),
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message;
    }
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

/// Updates the context line with the selection's train/build options. The
/// options only appear for the local player's own producers — a selected
/// enemy building shows what it is, not orders it would refuse.
pub fn update_help(
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    producers: Query<(&EntityInfoComponent, Option<&OwnerComponent>)>,
    mut text: Query<&mut Text, With<HelpText>>,
) {
    let mut message = String::from(
        "LMB select (Shift add, dbl-click all of type) | RMB move/harvest/attack | F/R/G/Q orders | X stance | 1-0 groups (Ctrl set) | V reveal | F1 debug | F2 spawn",
    );

    let local = session.local_player();
    if let Some(&id) = selection.get(local).first()
        && let Some((info, _)) = producers.iter().find(|(info, owner)| {
            info.id() == id && owner.is_some_and(|owner| owner.player() == local)
        })
    {
        let def = registry.def(info.type_id());
        if def.trainer.is_some() {
            message = String::from(
                "Train: click a command-card button   |   RMB set rally (on self clears)",
            );
        }
        if def.builder.is_some() {
            message =
                String::from("Build: click a command-card button   |   click the map to place");
        }
        if !def.skills.is_empty() {
            message = String::from("Skill: click a command-card button");
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
    registry: Res<ContentRegistry>,
    entities: Query<(
        &EntityInfoComponent,
        Option<&HealthComponent>,
        Option<&ResourceCarrierComponent>,
        Option<&ResourceSourceComponent>,
        Option<&StanceComponent>,
        Option<&EnergyComponent>,
        Option<&BuffsComponent>,
    )>,
    mut text: Query<&mut Text, With<SelectionText>>,
) {
    let selected = selection.get(session.local_player());
    let message = match selected {
        [] => String::new(),
        [id] => entities
            .iter()
            .find(|(info, ..)| info.id() == *id)
            .map(|(info, health, carrier, source, stance, energy, buffs)| {
                let def = registry.def(info.type_id());
                let mut parts = vec![pretty_name(info.type_name())];
                if let (Some(health), Some(max_health)) =
                    (health, def.base_stat(StatId::MAX_HEALTH))
                {
                    parts.push(format!(
                        "HP {}/{}",
                        health.displayed(),
                        max_health.to_num::<u32>()
                    ));
                }
                if let Some(carrier) = carrier
                    && let Some(kind) = &carrier.kind
                {
                    parts.push(format!("carrying {} {kind}", carrier.amount));
                }
                if let (Some(source), Some(source_def)) = (source, def.resource_source.as_ref()) {
                    parts.push(format!("{} {} left", source.amount, source_def.kind()));
                }
                if let Some(StanceComponent(stance)) = stance {
                    parts.push(format!("stance: {}", stance.name().replace('_', " ")));
                }
                if let Some(energy) = energy {
                    parts.push(format!("energy {}", energy.current_as_u32()));
                }
                if let Some(buffs) = buffs
                    && !buffs.is_empty()
                {
                    let names: Vec<String> = buffs
                        .active()
                        .map(|(id, stacks)| {
                            let name = pretty_name(registry.buff_name(id).unwrap_or("buff"));
                            if stacks > 1 {
                                format!("{name} x{stacks}")
                            } else {
                                name
                            }
                        })
                        .collect();
                    parts.push(names.join(", "));
                }
                parts.join("   ")
            })
            .unwrap_or_default(),
        many => format!("{} units selected", many.len()),
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message;
    }
}

/// Refreshes the scenario objectives checklist. Blank outside a scripted
/// mission, so an ordinary game shows nothing.
pub fn update_objectives(
    objectives: Option<Res<ScenarioObjectives>>,
    mut text: Query<&mut Text, With<ObjectivesText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let message = match objectives {
        Some(objectives) if !objectives.0.is_empty() => {
            let mut lines = vec![String::from("Objectives:")];
            lines.extend(objectives.0.iter().map(|objective| {
                format!(
                    "[{}] {}",
                    if objective.done { "x" } else { " " },
                    objective.label
                )
            }));
            lines.join("\n")
        }
        _ => String::new(),
    };
    **text = message;
}

/// Shows a Victory/Defeat/Draw banner once the session has finished.
pub fn update_game_over(session: Res<GameSession>, mut text: Query<&mut Text, With<GameOverText>>) {
    let message = match session.result() {
        None => "",
        Some(GameResult::Draw) => "Draw",
        Some(GameResult::Desynchronization { .. }) => "Desynchronization!",
        Some(GameResult::Aborted) => "Aborted",
        Some(GameResult::Defeat) => "Defeat",
        // Victory for the winning side; every other player sees a defeat.
        Some(GameResult::Victory { winner })
            if session.is_winner(session.local_player(), winner) =>
        {
            "Victory!"
        }
        Some(GameResult::Victory { .. }) => "Defeat",
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message.to_string();
    }
}

/// The shared visual bundle for a command-card button labelled `label`, tinted
/// with its resting `base` colour.
fn card_button(label: &str, base: Color) -> impl Bundle {
    (
        Button,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(base),
        children![(
            Text::new(label.to_string()),
            TextFont {
                font_size: 15.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.95)),
        )],
    )
}

/// Rebuilds the command card whenever the primary selection changes: a train
/// button per unit the selected producer can build, a build button per building
/// the selected worker can construct, or nothing when the primary does neither.
pub fn update_command_card(
    primary: Res<Primary>,
    registry: Res<ContentRegistry>,
    entities: Query<&EntityInfoComponent>,
    card: Query<Entity, With<CommandCard>>,
    buttons: Query<Entity, Or<(With<TrainButton>, With<BuildButton>, With<SkillButton>)>>,
    mut commands: Commands,
) {
    if !primary.is_changed() {
        return;
    }
    let Ok(card) = card.single() else {
        return;
    };
    for button in &buttons {
        commands.entity(button).despawn();
    }
    let Some(id) = primary.0 else {
        return;
    };
    let def = entities
        .iter()
        .find(|info| info.id() == id)
        .map(|info| registry.def(info.type_id()));
    let trains = def
        .and_then(|def| def.trainer.as_ref())
        .map(|trainer| trainer.trains().map(String::from).collect::<Vec<_>>())
        .unwrap_or_default();
    let builds = def
        .and_then(|def| def.builder.as_ref())
        .map(|builder| builder.builds().map(String::from).collect::<Vec<_>>())
        .unwrap_or_default();
    let skills: Vec<(SkillId, String)> = def
        .map(|def| {
            def.skills
                .iter()
                .map(|&id| (id, pretty_name(registry.skill_name(id).unwrap_or("skill"))))
                .collect()
        })
        .unwrap_or_default();

    commands.entity(card).with_children(|parent| {
        for name in trains {
            parent.spawn((
                TrainButton {
                    type_name: name.clone(),
                },
                card_button(&pretty_name(&name), BUTTON_NORMAL),
            ));
        }
        for name in builds {
            parent.spawn((
                BuildButton {
                    type_name: name.clone(),
                },
                card_button(&pretty_name(&name), BUILD_NORMAL),
            ));
        }
        for (id, label) in skills {
            parent.spawn((SkillButton { skill: id }, card_button(&label, SKILL_NORMAL)));
        }
    });
}

/// Trains the button's unit on the primary producer when a train button is clicked.
pub fn command_card_input(
    mut buttons: Query<(&Interaction, &TrainButton, &mut BackgroundColor), Changed<Interaction>>,
    primary: Res<Primary>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                if let Some(trainer) = primary.0 {
                    pending.push(PlayerCommand::TrainEntity {
                        trainer,
                        type_name: button.type_name.clone(),
                    });
                }
            }
            Interaction::Hovered => *color = BackgroundColor(BUTTON_HOVERED),
            Interaction::None => *color = BackgroundColor(BUTTON_NORMAL),
        }
    }
}

/// Starts placing the button's building when a build button is clicked; the
/// existing placement flow then handles the ghost and the confirming click.
pub fn build_card_input(
    mut buttons: Query<(&Interaction, &BuildButton, &mut BackgroundColor), Changed<Interaction>>,
    mut mode: ResMut<InputMode>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                *mode = InputMode::PlacingBuild(button.type_name.clone());
            }
            Interaction::Hovered => *color = BackgroundColor(BUILD_HOVERED),
            Interaction::None => *color = BackgroundColor(BUILD_NORMAL),
        }
    }
}

/// Casts the button's skill on every selected unit when clicked.
pub fn skill_card_input(
    mut buttons: Query<(&Interaction, &SkillButton, &mut BackgroundColor), Changed<Interaction>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                for &caster in selection.get(session.local_player()) {
                    pending.push(PlayerCommand::UseSkill {
                        caster,
                        skill: button.skill,
                        target: None,
                    });
                }
            }
            Interaction::Hovered => *color = BackgroundColor(SKILL_HOVERED),
            Interaction::None => *color = BackgroundColor(SKILL_NORMAL),
        }
    }
}

/// Rebuilds the control-group roster when the groups change: a chip per non-empty
/// group of the local player, labelled with its recall key and member count.
pub fn update_group_roster(
    session: Res<GameSession>,
    groups: Res<ControlGroups>,
    roster: Query<Entity, With<GroupRoster>>,
    chips: Query<Entity, With<GroupButton>>,
    mut commands: Commands,
) {
    if !groups.is_changed() {
        return;
    }
    let Ok(roster) = roster.single() else {
        return;
    };
    for chip in &chips {
        commands.entity(chip).despawn();
    }
    let local = session.local_player();
    commands.entity(roster).with_children(|parent| {
        for group in 0..CONTROL_GROUP_COUNT {
            let count = groups.get(local, group).len();
            if count == 0 {
                continue;
            }
            // The recall key is a client concern: group index 0..9 maps to keys 1..9,0.
            let key = (group + 1) % CONTROL_GROUP_COUNT;
            parent.spawn((
                GroupButton { group: group as u8 },
                card_button(&format!("{key}: {count}"), BUTTON_NORMAL),
            ));
        }
    });
}

/// Recalls a group when its roster chip is clicked.
pub fn group_roster_input(
    mut chips: Query<(&Interaction, &GroupButton, &mut BackgroundColor), Changed<Interaction>>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, chip, mut color) in &mut chips {
        match interaction {
            Interaction::Pressed => pending.push(PlayerCommand::RecallGroup {
                group: chip.group,
                mode: SelectMode::Replace,
            }),
            Interaction::Hovered => *color = BackgroundColor(BUTTON_HOVERED),
            Interaction::None => *color = BackgroundColor(BUTTON_NORMAL),
        }
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
