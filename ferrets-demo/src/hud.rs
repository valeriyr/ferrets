//! Minimal HUD: a resource bar and a context line for the current selection.

use std::collections::BTreeSet;

use bevy::prelude::*;
use ferrets_bevy_plugin::{PendingInput, ReplayPlayback, ScenarioObjectives};
use ferrets_content::{
    entity_stats::EntityStatId,
    registry::ContentRegistry,
    research::ResearchId,
    skills::{EntityCastTarget, SkillCaster, SkillId},
};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_physics::body;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef},
    components::{
        build::UnderConstructionComponent,
        energy::EnergyComponent,
        entity_buffs::BuffsComponent,
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        location::LocationComponent,
        order_queue::OrderQueueComponent,
        owner::OwnerComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::StanceComponent,
    },
    control_groups::{CONTROL_GROUP_COUNT, ControlGroups},
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    fields::{self, FieldGrid},
    game_loop::orders,
    order::Order,
    player_research::PlayerResearch,
    player_skills::PlayerSkills,
    requirements,
    resources::PlayerResources,
    selection::Selection,
    session::{
        GameResult, GameSession, Winner, local_role::LocalRole, player_id::PlayerId,
        player_slot::Participation,
    },
    simulation_id::SimulationId,
    statistics::{PlayerTally, Statistics},
    supply,
};

use crate::{
    input::{InputMode, Primary, TargetedOrder},
    states::{GameState, InGameUi},
    time::SpeedStep,
};

const BUTTON_NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const BUTTON_HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);
// Build buttons get a cooler tint so they stay distinct from train buttons on an
// entity that can do both.
const BUILD_NORMAL: Color = Color::srgb(0.16, 0.22, 0.30);
const BUILD_HOVERED: Color = Color::srgb(0.24, 0.32, 0.44);
// Skill buttons get a warm violet tint so abilities read apart from train/build.
const SKILL_NORMAL: Color = Color::srgb(0.26, 0.18, 0.30);
const SKILL_HOVERED: Color = Color::srgb(0.38, 0.26, 0.44);
// Research buttons get a teal tint so upgrades read apart from everything else.
const RESEARCH_NORMAL: Color = Color::srgb(0.14, 0.28, 0.26);
const RESEARCH_HOVERED: Color = Color::srgb(0.20, 0.40, 0.38);
// A produce/research button whose action the executor would refuse right now —
// requirements unmet, or the research already done or under way.
const CARD_DISABLED: Color = Color::srgb(0.12, 0.12, 0.13);
// The supply readout turns red the moment there is no headroom left.
const SUPPLY_NORMAL: Color = Color::srgb(0.85, 0.9, 0.85);
const SUPPLY_BLOCKED: Color = Color::srgb(1.0, 0.35, 0.3);

#[derive(Component)]
pub struct ResourceText;

/// The supply readout, red while training is supply-blocked.
#[derive(Component)]
pub struct SupplyText;

#[derive(Component)]
pub struct HelpText;

#[derive(Component)]
pub struct SelectionText;

#[derive(Component)]
pub struct GameOverText;

/// Marks the per-player tallies shown beside the game-over banner.
#[derive(Component)]
pub struct FinalStatsText;

#[derive(Component)]
pub struct SpectatorText;

#[derive(Component)]
pub struct RosterText;

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

/// A command-card button that starts a research on the primary researcher.
#[derive(Component)]
pub struct ResearchButton {
    /// The research this button starts.
    research: ResearchId,
}

/// A command-card button that starts one of the selection's declared
/// transitions.
#[derive(Component)]
pub struct MorphButton {
    /// The type the button changes into.
    type_name: String,
}

/// A command-card button that casts a skill on the selection.
#[derive(Component)]
pub struct PlayerSkillButton {
    /// The player-cast skill this button casts.
    skill: SkillId,
}

/// A command-card button that casts a skill on the selection.
#[derive(Component)]
pub struct SkillButton {
    /// The skill this button casts.
    skill: SkillId,
}

/// A command-card button that unloads the primary transporter's passengers.
#[derive(Component)]
pub struct UnloadButton {
    /// `false` unloads in place; `true` arms a click that names the
    /// destination.
    at_point: bool,
}

/// A command-card button that arms a click naming the unit the primary
/// transporter fetches aboard.
#[derive(Component)]
pub struct LoadButton;

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
pub fn setup_hud(mut commands: Commands, registry: Res<ContentRegistry>) {
    // The player-cast rallying call: one persistent button atop the bottom-left
    // control cluster (help line, command card, group roster), clear of the
    // leave button in the opposite corner.
    if let Some(war_drums) = registry.skill("war_drums") {
        commands
            .spawn((
                InGameUi,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(140.0),
                    left: Val::Px(10.0),
                    ..default()
                },
            ))
            .with_children(|parent| {
                parent.spawn((
                    PlayerSkillButton { skill: war_drums },
                    card_button("War Drums", SKILL_NORMAL),
                ));
            });
    }

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
        SupplyText,
        Text::new("Supply: 0/0"),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(SUPPLY_NORMAL),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(34.0),
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
            // Ends clear of the Leave button in the bottom-right, so the line
            // wraps instead of running underneath it on a narrow window.
            right: Val::Px(120.0),
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
    // Scenario objectives, top-left — dropped well below the resource bar and
    // debug readout so the checklist reads as its own block. Empty (and invisible)
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
            top: Val::Px(130.0),
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
    // What each player did, under the banner and shown with it.
    commands.spawn((
        InGameUi,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            top: Val::Percent(50.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            FinalStatsText,
            Text::new(""),
            TextFont {
                font_size: 15.0,
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.88, 0.9)),
        )],
    ));
    // The standing note that this node only watches: an observer from the
    // start, or a defeated player spectating on. Top-center, clear of the
    // resource bar; empty while the local player still plays.
    commands.spawn((
        InGameUi,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            // Its own band: under the resource/supply lines (top 8 and 34)
            // and the debug readout (top 60), aligned with neither.
            top: Val::Px(84.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            SpectatorText,
            Text::new(""),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.85, 0.6)),
        )],
    ));
    // Who is in the match and how they stand — the spectator's map of whom
    // they are watching, and every player's glance at the field.
    commands.spawn((
        InGameUi,
        RosterText,
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgb(0.75, 0.8, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(30.0),
            right: Val::Px(12.0),
            ..default()
        },
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
            bottom: Val::Px(106.0),
            left: Val::Px(10.0),
            column_gap: Val::Px(6.0),
            ..default()
        },
    ));
    // Command card: train/build buttons for the selected producer, sitting
    // clear above the help line — two rows of hint plus one wrapped row on a
    // narrow window — so the hint never runs into the buttons.
    commands.spawn((
        InGameUi,
        CommandCard,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(72.0),
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
/// playback freezes, or a divergence warning if a recorded checksum failed. A
/// paused or rescaled game says so too, whichever it is.
pub fn update_replay_note(
    playback: Option<Res<ReplayPlayback>>,
    session: Res<GameSession>,
    mut text: Query<&mut Text, With<ReplayNote>>,
) {
    // The speed named is the session's own, so a change a peer made — which never
    // touched this node's keys — reads the same as one made here.
    let speed = SpeedStep::of(session.speed());
    let message = match playback {
        Some(playback) if playback.mismatch().is_some() => {
            format!("Replay diverged at tick {}", playback.mismatch().unwrap())
        }
        Some(playback) if playback.is_done() => String::from("Replay ended"),
        _ if session.is_paused() => String::from("Paused"),
        _ if speed != SpeedStep::Normal => format!("Speed {}", speed.label()),
        _ => String::new(),
    };

    // Written only on change: the paused and rescaled notes stand for minutes at
    // a time, and rewriting the text re-shapes it every frame.
    if let Ok(mut text) = text.single_mut()
        && **text != message
    {
        **text = message;
    }
}

/// Updates the resource bar from the local player's stockpile.
pub fn update_resources(
    resources: Res<PlayerResources>,
    session: Res<GameSession>,
    mut text: Query<&mut Text, With<ResourceText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    // A watcher has no stockpile to read out.
    **text = match session.local_player() {
        None => String::new(),
        Some(player) => format!(
            "Gold: {}   Wood: {}",
            resources.amount(player, "gold"),
            resources.amount(player, "wood"),
        ),
    };
}

/// Updates the supply readout with the local player's used/provided totals,
/// turning it red while there is no headroom left (run in `Update`).
///
/// Exclusive, because the derived totals read entities and resources across the
/// whole world.
pub fn update_supply(world: &mut World) {
    // A watcher has no supply of its own to read out — the line is cleared
    // outright, or the element's spawn-time placeholder would linger.
    let readout = world
        .resource::<GameSession>()
        .local_player()
        .map(|player| {
            let provided = supply::provided(world, player).to_num::<u32>();
            let used = supply::used(world, player).to_num::<u32>();
            (format!("Supply: {used}/{provided}"), used >= provided)
        });

    let mut query = world.query_filtered::<(&mut Text, &mut TextColor), With<SupplyText>>();
    let Ok((mut text, mut color)) = query.single_mut(world) else {
        return;
    };
    match readout {
        Some((line, blocked)) => {
            **text = line;
            *color = TextColor(if blocked {
                SUPPLY_BLOCKED
            } else {
                SUPPLY_NORMAL
            });
        }
        None => text.clear(),
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
        "LMB select (Shift add, dbl-click all of type) | RMB move/harvest/attack | F/R/G/T/B/Q orders | X stance | 1-0 groups (Ctrl set)\nMinimap: LMB look (drag pans), RMB order | V reveal | P pause | -/= speed | . step | ] seek | M sound | F1 debug | F2 spawn | F3 layer",
    );

    if let Some(local) = session.local_player()
        && let Some(&id) = selection.get(local).first()
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
    fields: Res<FieldGrid>,
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&OwnerComponent>,
        Option<&HealthComponent>,
        Option<&StatsComponent>,
        Option<&ResourceCarrierComponent>,
        Option<&ResourceSourceComponent>,
        Option<&StanceComponent>,
        Option<&EnergyComponent>,
        Option<&BuffsComponent>,
        Option<&UnderConstructionComponent>,
    )>,
    inspected: Res<crate::input::Inspected>,
    mut text: Query<&mut Text, With<SelectionText>>,
) {
    // A playing node's panel shows its live selection; a watching one's —
    // by role, or a player whose defeat took effect — shows what it picked
    // to look at. One slice either way, so the readout below serves both.
    let selected: &[SimulationId] = match session.local_role() {
        LocalRole::Player(local) if session.is_player_live(local) => selection.get(local),
        LocalRole::Player(_) | LocalRole::Observer => &inspected.0,
    };
    let message = match selected {
        [] => String::new(),
        [id] => entities
            .iter()
            .find(|(info, ..)| info.id() == *id)
            .map(
                |(
                    info,
                    location,
                    owner,
                    health,
                    stats,
                    carrier,
                    source,
                    stance,
                    energy,
                    buffs,
                    under_construction,
                )| {
                    let def = registry.def(info.type_id());
                    // The simulation id rides along with the name: it is the
                    // handle a replay, a log line, or a forensics run names the
                    // same entity by, so a report can point at one unit rather
                    // than describe it.
                    let mut parts = vec![format!(
                        "{} #{}",
                        pretty_name(info.type_name()),
                        info.id().0
                    )];
                    // The effective ceiling, so a modifier that moves max health shows in
                    // the denominator instead of leaving the reading out of step with it.
                    let max_health = stats
                        .and_then(|stats| stats.effective(EntityStatId::MAX_HEALTH))
                        .or_else(|| def.base_stat(EntityStatId::MAX_HEALTH));
                    if let (Some(health), Some(max_health)) = (health, max_health) {
                        parts.push(format!(
                            "HP {}/{}",
                            health.displayed(),
                            max_health.to_num::<u32>()
                        ));
                    }
                    // Effective values, so an upgrade or a running buff shows
                    // in the reading the moment it lands.
                    if let Some(damage) = stats
                        .and_then(|stats| stats.effective(EntityStatId::DAMAGE))
                        .or_else(|| def.base_stat(EntityStatId::DAMAGE))
                    {
                        parts.push(format!("attack {}", damage.to_num::<u32>()));
                    }
                    if let Some(speed) = stats
                        .and_then(|stats| stats.effective(EntityStatId::SPEED))
                        .or_else(|| def.base_stat(EntityStatId::SPEED))
                    {
                        parts.push(format!("speed {:.2}", speed.to_num::<f32>()));
                    }
                    if let Some(carrier) = carrier
                        && let Some(kind) = &carrier.kind
                    {
                        parts.push(format!("carrying {} {kind}", carrier.amount));
                    }
                    if let (Some(source), Some(source_def)) = (source, def.resource_source.as_ref())
                    {
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
                                let name =
                                    pretty_name(registry.entity_buff_name(id).unwrap_or("buff"));
                                if stacks > 1 {
                                    format!("{name} x{stacks}")
                                } else {
                                    name
                                }
                            })
                            .collect();
                        parts.push(names.join(", "));
                    }
                    // Not operating: still going up, or standing outside the
                    // field it needs. Construction wins, as it does for the
                    // engine's own reading.
                    let disabled = fields::disabled_in(
                        &fields,
                        &session,
                        def,
                        owner.map(|owner| owner.player()),
                        body::anchor(location.position),
                    );
                    match (under_construction, disabled) {
                        (Some(_), _) => parts.push("under construction".to_string()),
                        (None, true) => parts.push("disabled".to_string()),
                        (None, false) => {}
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

/// Shows the standing can't-play note while the local player is a spectator:
/// "Observing" for an observer seat; for an eliminated player, defeat wording
/// only once its whole side is out — eliminated alone, it watches allies who
/// may still win for it. Cleared once the game ends — the verdict banner
/// takes over.
pub fn update_spectator_note(
    session: Res<GameSession>,
    watch: Res<crate::render::ObserverPerspective>,
    mut text: Query<&mut Text, With<SpectatorText>>,
) {
    let message = if session.result().is_some() {
        // The verdict banner has taken over.
        String::new()
    } else {
        match session.local_role() {
            // This node watches by role, through whichever perspective it
            // flipped to — a side's view, named by its team when it has one.
            LocalRole::Observer => match watch.0 {
                None => "Observing - everything (Tab: next view)".to_string(),
                Some(side) => match session.slot(side).and_then(|slot| slot.team()) {
                    Some(team) => format!("Observing - team {team}'s view (Tab: next view)"),
                    None => format!("Observing - player {side}'s view (Tab: next view)"),
                },
            },
            // A player watching is one whose elimination has taken effect —
            // called a defeat only once no ally plays on, since a side still
            // standing can still win for it. A playing player gets no note —
            // and neither does a dropped one, whose ending (`Aborted`) is the
            // network layer's to announce.
            LocalRole::Player(local) if session.is_player_eliminated(local) => {
                let side_plays_on = session.player_slots().any(|slot| {
                    session.are_allied(local, slot.id()) && session.is_player_live(slot.id())
                });
                if side_plays_on {
                    "Eliminated - spectating (your side plays on)".to_string()
                } else {
                    "Defeat - spectating".to_string()
                }
            }
            LocalRole::Player(_) => String::new(),
        }
    };
    if let Ok(mut text) = text.single_mut() {
        **text = message;
    }
}

/// Refreshes the player roster: every seat in the match and how it stands.
/// The environment's combatants are scenery, not participants, so they stay
/// off the list.
pub fn update_roster(session: Res<GameSession>, mut text: Query<&mut Text, With<RosterText>>) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let local = session.local_player();
    let lines: Vec<String> = session
        .occupied_slots()
        .filter(|slot| slot.participation() != Some(Participation::Environment))
        .map(|slot| {
            let id = slot.id();
            let team = slot
                .team()
                .map_or(String::new(), |team| format!(" [T{team}]"));
            let standing = if session.is_player_dropped(id) {
                "left"
            } else if session.is_player_eliminated(id) {
                "eliminated"
            } else {
                "playing"
            };
            let you = if Some(id) == local { " (you)" } else { "" };
            format!("P{id}{team}: {standing}{you}")
        })
        .collect();
    **text = lines.join("\n");
}

/// Shows a Victory/Defeat/Draw banner once the session has finished.
pub fn update_game_over(session: Res<GameSession>, mut text: Query<&mut Text, With<GameOverText>>) {
    let message = match session.result() {
        None => "",
        Some(GameResult::Draw) => "Draw",
        Some(GameResult::Desynchronization { .. }) => "Desynchronization!",
        Some(GameResult::Aborted) => "Aborted",
        Some(GameResult::Defeat) => "Defeat",
        // Victory for the winning side; every other player sees a defeat —
        // and a watcher, on nobody's side, sees the verdict itself.
        Some(GameResult::Victory { winner }) => match session.local_player() {
            None => {
                return if let Ok(mut text) = text.single_mut() {
                    **text = match winner {
                        Winner::Team(team) => format!("Team {team} wins"),
                        Winner::Player(player) => format!("Player {player} wins"),
                    };
                };
            }
            Some(local) if session.is_winner(local, winner) => "Victory!",
            Some(_) => "Defeat",
        },
    };

    if let Ok(mut text) = text.single_mut() {
        **text = message.to_string();
    }
}

/// Fills in what each seated player did, once the session has finished — every
/// player's, not only the local one; free seats and environment combatants get
/// no row. Empty while a game is still running.
///
/// The report is rebuilt only when it can have changed — the first frame after
/// the result lands, and any frame the tallies still move (a defeated player
/// spectating a game that plays on) — and written only when it differs.
pub fn update_final_statistics(
    session: Res<GameSession>,
    statistics: Res<Statistics>,
    mut text: Query<&mut Text, With<FinalStatsText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    if session.result().is_none() {
        if !text.is_empty() {
            **text = String::new();
        }
        return;
    }
    if !text.is_empty() && !statistics.is_changed() {
        return;
    }

    let local = session.local_player();
    let lines: Vec<String> = session
        .slots()
        .iter()
        .filter(|slot| matches!(slot.participation(), Some(Participation::Player)))
        .map(|slot| {
            let id = slot.id();
            let tally = statistics.player(id);
            let you = if Some(id) == local { " (you)" } else { "" };
            // Per-type counts summed for the headline: the engine keeps the
            // breakdown, and what it is worth is the game's business.
            let produced: u32 = tally.produced_types().map(|(_, count)| count).sum();
            let lost: u32 = tally.lost_types().map(|(_, count)| count).sum();
            let killed: u32 = tally.killed_types().map(|(_, count)| count).sum();
            format!(
                "P{id}{you}: built {produced}, lost {lost}, killed {killed}, \
                 damage {dealt}/{taken}, research {research}, skills {skills}\n\
                 {spacer}{economy}",
                dealt = tally.damage_dealt().to_num::<u32>(),
                taken = tally.damage_taken().to_num::<u32>(),
                research = tally.research_completed(),
                skills = tally.skills_cast(),
                spacer = " ".repeat(4),
                economy = economy_line(tally),
            )
        })
        .collect();
    let report = lines.join("\n");
    if **text != report {
        **text = report;
    }
}

/// One player's resource flow, by kind: what came in, what went out, and what
/// came back. Kinds with no movement at all are left out.
fn economy_line(tally: &PlayerTally) -> String {
    let mut kinds: BTreeSet<&str> = BTreeSet::new();
    kinds.extend(tally.gathered_kinds().map(|(kind, _)| kind));
    kinds.extend(tally.spent_kinds().map(|(kind, _)| kind));
    kinds.extend(tally.refunded_kinds().map(|(kind, _)| kind));
    if kinds.is_empty() {
        return "no economy".to_string();
    }
    kinds
        .into_iter()
        .map(|kind| {
            let refunded = tally.refunded(kind);
            let back = if refunded > 0 {
                format!(" (back {refunded})")
            } else {
                String::new()
            };
            format!(
                "{kind} +{gathered}/-{spent}{back}",
                gathered = tally.gathered(kind),
                spent = tally.spent(kind),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
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

/// Rebuilds the command card whenever the primary selection changes — a train
/// button per unit the selected producer can build, a build button per building
/// the selected worker can construct, or nothing when the primary does neither
/// — and whenever the primary's own type is rewritten: a gryphon that takes
/// off must swap its take-off button for the landing one on the spot.
pub fn update_command_card(
    session: Res<GameSession>,
    primary: Res<Primary>,
    registry: Res<ContentRegistry>,
    changed: Query<&EntityInfoComponent, Changed<EntityInfoComponent>>,
    entities: Query<&EntityInfoComponent>,
    card: Query<Entity, With<CommandCard>>,
    buttons: Query<
        Entity,
        Or<(
            With<TrainButton>,
            With<BuildButton>,
            With<ResearchButton>,
            With<SkillButton>,
            With<UnloadButton>,
            With<LoadButton>,
            With<MorphButton>,
        )>,
    >,
    mut commands: Commands,
) {
    // A spectator commands nothing, so it gets no command card at all; the
    // buttons of the seat it played before its defeat despawn here too.
    if !session.local_plays() {
        for button in &buttons {
            commands.entity(button).despawn();
        }
        return;
    }
    let primary_type_changed = primary
        .0
        .is_some_and(|id| changed.iter().any(|info| info.id() == id));
    if !primary.is_changed() && !primary_type_changed {
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
    let researches: Vec<(ResearchId, String)> = def
        .and_then(|def| def.researcher.as_ref())
        .map(|researcher| {
            researcher
                .researches()
                .map(|id| {
                    (
                        id,
                        pretty_name(registry.research_name(id).unwrap_or("research")),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let skills: Vec<(SkillId, String)> = def
        .map(|def| {
            def.skills
                .iter()
                .map(|&id| (id, pretty_name(registry.skill_name(id).unwrap_or("skill"))))
                .collect()
        })
        .unwrap_or_default();
    let transports = def.is_some_and(|def| def.can_transport());
    let morphs: Vec<String> = def
        .map(|def| {
            def.morphs
                .iter()
                .map(|transition| transition.into_type().to_string())
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
        for (id, label) in researches {
            parent.spawn((
                ResearchButton { research: id },
                card_button(&label, RESEARCH_NORMAL),
            ));
        }
        for (id, label) in skills {
            parent.spawn((SkillButton { skill: id }, card_button(&label, SKILL_NORMAL)));
        }
        for name in morphs {
            parent.spawn((
                card_button(&pretty_name(&name), SKILL_NORMAL),
                MorphButton { type_name: name },
            ));
        }
        if transports {
            parent.spawn((LoadButton, card_button("Load", BUTTON_NORMAL)));
            parent.spawn((
                UnloadButton { at_point: false },
                card_button("Unload", BUTTON_NORMAL),
            ));
            parent.spawn((
                UnloadButton { at_point: true },
                card_button("Unload To", BUTTON_NORMAL),
            ));
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

/// Arms the fetch-aboard click when the load button is clicked.
pub fn load_card_input(
    mut buttons: Query<&Interaction, (With<LoadButton>, Changed<Interaction>)>,
    primary: Res<Primary>,
    mut mode: ResMut<InputMode>,
) {
    for interaction in &mut buttons {
        if matches!(interaction, Interaction::Pressed) && primary.0.is_some() {
            *mode = InputMode::Targeting(TargetedOrder::Load);
        }
    }
}

/// Unloads the primary transporter when an unload button is clicked: in place,
/// or — for the at-point button — by arming a click that names the destination.
pub fn unload_card_input(
    mut buttons: Query<(&Interaction, &UnloadButton), Changed<Interaction>>,
    primary: Res<Primary>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button) in &mut buttons {
        if !matches!(interaction, Interaction::Pressed) {
            continue;
        }
        let Some(transport) = primary.0 else {
            continue;
        };
        if button.at_point {
            *mode = InputMode::Targeting(TargetedOrder::Unload);
        } else {
            pending.push(PlayerCommand::Unload {
                transport,
                at: None,
                flush: true,
            });
        }
    }
}

/// Changes the selection into the button's type when clicked.
///
/// The whole selection is commanded, not just the primary, because a mixed
/// selection reads naturally as "everyone that can, change" — the executor
/// drops whoever cannot.
pub fn morph_card_input(
    mut buttons: Query<(&Interaction, &MorphButton, &mut BackgroundColor), Changed<Interaction>>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                pending.push(PlayerCommand::Morph {
                    type_name: button.type_name.clone(),
                    flush: true,
                });
            }
            Interaction::Hovered => *color = BackgroundColor(BUTTON_HOVERED),
            Interaction::None => *color = BackgroundColor(SKILL_NORMAL),
        }
    }
}

/// Starts the button's research on the primary researcher when clicked. The
/// executor holds every gate (requirements, completion, the one-per-topic
/// rule), so a click that slips past the greyed-out tint is still refused.
pub fn research_card_input(
    mut buttons: Query<(&Interaction, &ResearchButton), Changed<Interaction>>,
    primary: Res<Primary>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button) in &mut buttons {
        if matches!(interaction, Interaction::Pressed)
            && let Some(researcher) = primary.0
        {
            pending.push(PlayerCommand::StartResearch {
                researcher,
                research: button.research,
            });
        }
    }
}

/// What a gated card button does when clicked — the part of it the executor
/// would judge, paired with the tints its kind rests and hovers in.
enum CardAction {
    /// Trains a unit of the type.
    Train(String),
    /// Places a building of the type.
    Build(String),
    /// Starts the research.
    Research(ResearchId),
    /// Casts the skill from the primary entity.
    Skill(SkillId),
    /// Casts the skill as the player.
    PlayerSkill(SkillId),
}

/// Recolors the gated card buttons from what the executor would currently
/// allow: a train, build, research or skill the primary entity may not start
/// now (see [`orders::can_start`]), or whose requirements are unmet, greys
/// out, as does a research that is done or already under way.
pub fn update_card_availability(world: &mut World) {
    // A watcher has no card to recolor — update_command_card despawned it.
    let Some(player) = world.resource::<GameSession>().local_player() else {
        return;
    };
    let primary = world
        .resource::<Primary>()
        .0
        .and_then(|id| world.resource::<EntityIndex>().interactable(world, id));
    let starts = |world: &World, order: Order| {
        primary.is_some_and(|entity| orders::can_start(world, entity, &order).is_ok())
    };
    let operating = |world: &World| {
        primary.is_some_and(|entity| {
            matches!(entity_def::operation(world, entity), Operation::Operating)
        })
    };

    let mut buttons: Vec<(Entity, Interaction, CardAction)> = Vec::new();
    let mut query = world.query::<(
        Entity,
        &Interaction,
        Option<&TrainButton>,
        Option<&BuildButton>,
        Option<&ResearchButton>,
        Option<&SkillButton>,
        Option<&PlayerSkillButton>,
    )>();
    for (entity, interaction, train, build, research, skill, player_skill) in query.iter(world) {
        let action = if let Some(button) = train {
            CardAction::Train(button.type_name.clone())
        } else if let Some(button) = build {
            CardAction::Build(button.type_name.clone())
        } else if let Some(button) = research {
            CardAction::Research(button.research)
        } else if let Some(button) = skill {
            CardAction::Skill(button.skill)
        } else if let Some(button) = player_skill {
            CardAction::PlayerSkill(button.skill)
        } else {
            continue;
        };
        buttons.push((entity, *interaction, action));
    }

    for (entity, interaction, action) in buttons {
        let type_requirements_met = |world: &mut World, type_name: &str| {
            world
                .resource::<ContentRegistry>()
                .entity(type_name)
                .map(|def| def.requires.clone())
                .is_none_or(|requires| requirements::met(world, player, &requires))
        };
        let skill_requirements_met = |world: &mut World, skill: SkillId| {
            world
                .resource::<ContentRegistry>()
                .skill_def(skill)
                .map(|def| def.requires.clone())
                .is_none_or(|requires| requirements::met(world, player, &requires))
        };
        let (available, normal, hovered) = match &action {
            CardAction::Train(type_name) => (
                type_requirements_met(world, type_name) && starts(world, Order::Train),
                BUTTON_NORMAL,
                BUTTON_HOVERED,
            ),
            CardAction::Build(type_name) => {
                // The site's cell is chosen after the click; the start check
                // reads the builder and the type, not the ground.
                let order = Order::Build {
                    type_name: type_name.clone(),
                    position: FixedUVec2::ZERO,
                };
                (
                    type_requirements_met(world, type_name) && starts(world, order),
                    BUILD_NORMAL,
                    BUILD_HOVERED,
                )
            }
            CardAction::Research(research) => {
                let available = !world
                    .resource::<PlayerResearch>()
                    .is_completed(player, *research)
                    && !research_under_way(world, player, *research)
                    && world
                        .resource::<ContentRegistry>()
                        .research_def(*research)
                        .map(|def| def.requires.clone())
                        .is_none_or(|requires| requirements::met(world, player, &requires))
                    && starts(
                        world,
                        Order::Research {
                            research: *research,
                        },
                    );
                (available, RESEARCH_NORMAL, RESEARCH_HOVERED)
            }
            CardAction::Skill(skill) => (
                skill_requirements_met(world, *skill) && operating(world),
                SKILL_NORMAL,
                SKILL_HOVERED,
            ),
            CardAction::PlayerSkill(skill) => (
                skill_requirements_met(world, *skill),
                SKILL_NORMAL,
                SKILL_HOVERED,
            ),
        };

        let color = match (available, interaction) {
            (false, _) => CARD_DISABLED,
            (true, Interaction::Hovered | Interaction::Pressed) => hovered,
            (true, Interaction::None) => normal,
        };
        if let Some(mut background) = world.entity_mut(entity).get_mut::<BackgroundColor>() {
            background.0 = color;
        }
    }
}

/// Whether any of the local player's entities is working on or queued for the
/// given research.
fn research_under_way(world: &mut World, player: PlayerId, research: ResearchId) -> bool {
    let mut query = world.query::<(&OwnerComponent, &OrderQueueComponent)>();
    query.iter(world).any(|(owner, queue)| {
        owner.player() == player
            && queue.0.iter().any(
                |entry| matches!(&entry.order, Order::Research { research: r } if *r == research),
            )
    })
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

/// Handles a click on a skill button: a self-cast fires for every selected
/// unit at once; a targeted skill arms the click that names the target.
pub fn skill_card_input(
    mut buttons: Query<(&Interaction, &SkillButton, &mut BackgroundColor), Changed<Interaction>>,
    registry: Res<ContentRegistry>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                let Some(def) = registry.skill_def(button.skill) else {
                    continue;
                };
                let target = match &def.caster {
                    SkillCaster::Entity { target, .. } => *target,
                    SkillCaster::Player { .. } => {
                        unreachable!("entity types declare only entity-cast skills")
                    }
                };
                match target {
                    // Self-cast: fires immediately for every selected unit that
                    // has the skill.
                    EntityCastTarget::Caster => {
                        let Some(local) = session.local_player() else {
                            continue;
                        };
                        for &caster in selection.get(local) {
                            pending.push(PlayerCommand::UseSkill {
                                skill: button.skill,
                                caster: SkillCasterRef::Entity(caster),
                                target: None,
                            });
                        }
                    }
                    // Targeted cast: arm the click that names the target.
                    EntityCastTarget::Ally
                    | EntityCastTarget::Enemy
                    | EntityCastTarget::Position => {
                        *mode = InputMode::Targeting(TargetedOrder::Skill(button.skill));
                    }
                }
            }
            Interaction::Hovered => *color = BackgroundColor(SKILL_HOVERED),
            Interaction::None => *color = BackgroundColor(SKILL_NORMAL),
        }
    }
}

/// Casts its button's player skill when clicked.
pub fn player_skill_card_input(
    mut buttons: Query<
        (&Interaction, &PlayerSkillButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut pending: ResMut<PendingInput>,
) {
    for (interaction, button, mut color) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                pending.push(PlayerCommand::UseSkill {
                    skill: button.skill,
                    caster: SkillCasterRef::Player,
                    target: None,
                });
            }
            Interaction::Hovered => *color = BackgroundColor(SKILL_HOVERED),
            Interaction::None => *color = BackgroundColor(SKILL_NORMAL),
        }
    }
}

/// Shows the player skill's cooldown on its button.
pub fn update_player_skill_cooldown(
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    skills: Res<PlayerSkills>,
    buttons: Query<(&PlayerSkillButton, &Children)>,
    mut texts: Query<&mut Text>,
) {
    // A watcher casts nothing, so there is no cooldown of its own to label.
    let Some(local) = session.local_player() else {
        return;
    };
    for (button, children) in &buttons {
        let name = pretty_name(registry.skill_name(button.skill).unwrap_or("skill"));
        let remaining = skills.cooldown_remaining(local, button.skill);
        let label = cooldown_label(name, remaining);
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.0 != label
            {
                text.0 = label.clone();
            }
        }
    }
}

/// Shows the primary selected entity's skill cooldowns on its command-card
/// buttons, the same way the player skill button shows its own.
pub fn update_skill_cooldowns(
    registry: Res<ContentRegistry>,
    primary: Res<Primary>,
    entities: Query<(&EntityInfoComponent, &SkillsComponent)>,
    buttons: Query<(&SkillButton, &Children)>,
    mut texts: Query<&mut Text>,
) {
    let Some(id) = primary.0 else {
        return;
    };
    let Some(skills) = entities
        .iter()
        .find(|(info, _)| info.id() == id)
        .map(|(_, skills)| skills)
    else {
        return;
    };
    for (button, children) in &buttons {
        let name = pretty_name(registry.skill_name(button.skill).unwrap_or("skill"));
        let label = cooldown_label(name, skills.cooldown_remaining(button.skill));
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.0 != label
            {
                text.0 = label.clone();
            }
        }
    }
}

/// A skill button's label: the bare name when ready, "name (Ns)" while the
/// cast recharges. Ticks are 20 Hz, so seconds are the remaining ticks over
/// twenty, rounded up.
fn cooldown_label(name: String, remaining: u32) -> String {
    match remaining {
        0 => name,
        ticks => format!("{name} ({}s)", ticks.div_ceil(20)),
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
    // A watcher holds no control groups; the chips stay despawned.
    let Some(local) = session.local_player() else {
        return;
    };
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
