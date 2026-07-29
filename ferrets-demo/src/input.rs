//! Mouse input → simulation commands: left-click/drag selection, right-click
//! move/send orders. Everything is issued as a `PlayerCommand` through
//! `PendingInput`, so it flows through the deterministic command pipeline.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ferrets_bevy_plugin::{NetworkActive, PauseIntent, PendingInput};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::entity_info::EntityInfoComponent,
    components::{
        hidden::HiddenComponent,
        location::LocationComponent,
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        stance::{Stance, StanceComponent},
    },
    content::registry::ContentRegistry,
    control_groups::ControlGroups,
    map::Map,
    selection::Selection,
    session::GameSession,
    simulation_id::SimulationId,
};

use crate::{camera, render::CELL_PX};

/// Drag below this many pixels is treated as a click, not a box-select.
const CLICK_SLOP: f32 = 4.0;

/// Window within which a repeat of the same action counts as a double input: a
/// second click on the same entity (select all of type) or a second press of the
/// same control-group key (recenter on the group).
const DOUBLE_CLICK_SECS: f32 = 0.35;

/// The last left-click on an entity (elapsed seconds, id), for double-click detection.
#[derive(Resource, Default)]
pub struct LastClick(Option<(f32, SimulationId)>);

/// Toggles pause on the `P` key. In a network game this records the local intent,
/// which the host turns into a tick-aligned pause every node applies together (a
/// client forwards it to the host); a local game pauses its session immediately.
pub fn pause_input(
    keys: Res<ButtonInput<KeyCode>>,
    networked: Option<Res<NetworkActive>>,
    mut intent: ResMut<PauseIntent>,
    mut session: ResMut<GameSession>,
) {
    if !keys.just_pressed(KeyCode::KeyP) {
        return;
    }
    let want_paused = !session.is_paused();
    if networked.is_some() {
        intent.0 = Some(want_paused);
    } else {
        session.set_paused(want_paused);
    }
}

/// What the next left-click does.
#[derive(Resource, Default)]
pub enum InputMode {
    /// Left-click selects.
    #[default]
    Normal,
    /// Left-click places the named building (started from the build menu).
    PlacingBuild(String),
    /// Left-click issues the armed combat order (started from a hotkey).
    Targeting(TargetedOrder),
}

/// A combat order armed by hotkey, waiting for its target click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetedOrder {
    /// `F` — attack-move to the clicked position.
    AttackMove,
    /// `R` — patrol between here and the clicked position.
    Patrol,
    /// `G` — guard the clicked entity.
    Guard,
}

/// World-space anchor of an in-progress left-drag.
#[derive(Resource, Default)]
pub struct DragStart(Option<Vec2>);

/// True when the cursor is over an interactive HUD element (a hovered or pressed
/// button), so a world-click system should leave the click to the UI rather than
/// acting on the map beneath it.
fn pointer_over_ui(interactions: &Query<&Interaction>) -> bool {
    interactions
        .iter()
        .any(|state| !matches!(state, Interaction::None))
}

/// The world position under the cursor, if any.
fn cursor_world(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> Option<Vec2> {
    let cursor = window.cursor_position()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

/// Converts a world position to its grid cell, or `None` if off the top/left edge.
fn world_to_cell(world: Vec2) -> Option<(u32, u32)> {
    let x = world.x / CELL_PX;
    let y = -world.y / CELL_PX;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    Some((x as u32, y as u32))
}

/// Cell as a `FixedUVec2` position (clamped to the non-negative quadrant).
fn world_to_pos(world: Vec2) -> FixedUVec2 {
    FixedUVec2::new(
        FixedU64::from_num((world.x / CELL_PX).max(0.0)),
        FixedU64::from_num((-world.y / CELL_PX).max(0.0)),
    )
}

/// A selection rectangle snapped to whole cells, spanning at least one cell, so
/// even a small drag covers the cells it crossed (and catches the integer-cell
/// positions of entities inside).
fn cell_rect(a: Vec2, b: Vec2) -> FixedURect {
    let (ax, bx) = ((a.x / CELL_PX).max(0.0), (b.x / CELL_PX).max(0.0));
    let (ay, by) = ((-a.y / CELL_PX).max(0.0), (-b.y / CELL_PX).max(0.0));
    let min_x = ax.min(bx).floor();
    let min_y = ay.min(by).floor();
    let max_x = ax.max(bx).ceil().max(min_x + 1.0);
    let max_y = ay.max(by).ceil().max(min_y + 1.0);
    FixedURect::from_corners(
        FixedUVec2::new(FixedU64::from_num(min_x), FixedU64::from_num(min_y)),
        FixedUVec2::new(FixedU64::from_num(max_x), FixedU64::from_num(max_y)),
    )
}

/// The cell rectangle currently visible in the viewport, for on-screen
/// select-all-of-type (a double-click grabs only what's on screen).
fn visible_cell_rect(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> FixedURect {
    let size = Vec2::new(window.width(), window.height());
    let corner = |v| {
        camera
            .viewport_to_world_2d(camera_transform, v)
            .unwrap_or(Vec2::ZERO)
    };
    cell_rect(corner(Vec2::ZERO), corner(size))
}

/// The selectable entity whose footprint covers `cell`, preferring the smallest
/// footprint (a unit standing on/near a building wins over the building).
fn entity_at(
    cell: (u32, u32),
    registry: &ContentRegistry,
    entities: &Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
) -> Option<SimulationId> {
    let mut best: Option<(u32, SimulationId)> = None;
    for (info, location) in entities {
        let ox = location.position.x.to_num::<u32>();
        let oy = location.position.y.to_num::<u32>();
        let size = registry.def(info.type_id()).location.unwrap().size();
        let inside =
            cell.0 >= ox && cell.0 < ox + size.width && cell.1 >= oy && cell.1 < oy + size.height;
        if inside {
            let area = size.width * size.height;
            if best.is_none_or(|(best_area, _)| area < best_area) {
                best = Some((area, info.id()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Left click selects a single entity; left drag box-selects a region. Disabled
/// while placing a building (left-click then places instead).
pub fn selection_input(
    mode: Res<InputMode>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    registry: Res<ContentRegistry>,
    mut drag: ResMut<DragStart>,
    mut last_click: ResMut<LastClick>,
    mut gizmos: Gizmos,
    mut pending: ResMut<PendingInput>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
) {
    if !matches!(*mode, InputMode::Normal) {
        // A drag interrupted by entering a mode must not fire when the click
        // that ends the mode is released back in Normal.
        drag.0 = None;
        return;
    }
    if pointer_over_ui(&interactions) {
        // A click on a HUD button belongs to the UI, not the map beneath it.
        drag.0 = None;
        return;
    }
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };

    if mouse.just_pressed(MouseButton::Left) {
        drag.0 = Some(cursor);
    }

    if let Some(start) = drag.0
        && mouse.pressed(MouseButton::Left)
        && start.distance(cursor) > CLICK_SLOP
    {
        // Live selection box.
        gizmos.rect_2d(
            Isometry2d::from_translation((start + cursor) / 2.0),
            (cursor - start).abs(),
            Color::srgb(0.2, 1.0, 0.4),
        );
    }

    if mouse.just_released(MouseButton::Left)
        && let Some(start) = drag.0.take()
    {
        // A click on an entity selects just that one; a click on empty ground or a
        // drag selects by rect, which finds nothing on empty ground and clears it.
        // Holding Shift combines with the current selection instead of replacing:
        // a click toggles the entity, a box adds its contents.
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if start.distance(cursor) <= CLICK_SLOP
            && let Some(cell) = world_to_cell(cursor)
            && let Some(id) = entity_at(cell, &registry, &entities)
        {
            let now = time.elapsed_secs();
            let double = last_click
                .0
                .is_some_and(|(t, last)| last == id && now - t <= DOUBLE_CLICK_SECS);
            last_click.0 = Some((now, id));

            // A double-click selects every on-screen entity of the same class;
            // a single click selects (or shift-toggles) just this one.
            let class = double
                .then(|| {
                    entities
                        .iter()
                        .find(|(info, ..)| info.id() == id)
                        .and_then(|(info, ..)| registry.entity(info.type_name()))
                        .map(|def| def.selection_class().to_string())
                })
                .flatten();
            if let Some(class) = class {
                let mode = if shift {
                    SelectMode::Add
                } else {
                    SelectMode::Replace
                };
                pending.push(PlayerCommand::SelectByType {
                    class,
                    rect: visible_cell_rect(window, camera, camera_transform),
                    mode,
                });
            } else {
                let mode = if shift {
                    SelectMode::Toggle
                } else {
                    SelectMode::Replace
                };
                pending.push(PlayerCommand::SelectById { id, mode });
            }
        } else {
            let mode = if shift {
                SelectMode::Add
            } else {
                SelectMode::Replace
            };
            pending.push(PlayerCommand::SelectByRect {
                rect: cell_rect(start, cursor),
                mode,
            });
        }
    }
}

/// Right click sends the selection to the entity under the cursor, or moves it
/// to the clicked cell. Holding Shift appends instead of replacing orders.
/// When the selection is entirely own producers, the click re-targets their
/// rally points instead — clicking one of the selected producers clears them.
pub fn order_input(
    mode: Res<InputMode>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    mut pending: ResMut<PendingInput>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
    rally_holders: Query<(&EntityInfoComponent, &OwnerComponent), With<RallyPointComponent>>,
) {
    if !mouse.just_pressed(MouseButton::Right) || !matches!(*mode, InputMode::Normal) {
        return;
    }
    if pointer_over_ui(&interactions) {
        // A right-click on a HUD button must not also order units on the map.
        return;
    }
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };

    let flush = !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));

    let target = world_to_cell(cursor).and_then(|cell| entity_at(cell, &registry, &entities));

    // Only an all-producer selection captures the click; a mixed selection
    // keeps ordering its units around normally.
    let local = session.local_player();
    let selected = selection.get(local);
    let all_producers = !selected.is_empty()
        && selected.iter().all(|&id| {
            rally_holders
                .iter()
                .any(|(info, owner)| info.id() == id && owner.player() == local)
        });
    if all_producers {
        let target = match target {
            Some(id) if selected.contains(&id) => None,
            Some(id) => Some(RallyTarget::Entity(id)),
            None => Some(RallyTarget::Position(world_to_pos(cursor))),
        };
        for &producer in selected {
            pending.push(PlayerCommand::SetRallyPoint {
                entity: producer,
                target,
            });
        }
        return;
    }

    match target {
        Some(target) => pending.push(PlayerCommand::SendToEntity { target, flush }),
        None => pending.push(PlayerCommand::Move {
            target: world_to_pos(cursor),
            flush,
        }),
    }
}

/// The local player's primary selected entity — the one hotkeys and panels act on.
#[derive(Resource, Default, PartialEq, Eq)]
pub struct Primary(pub Option<SimulationId>);

/// Recomputes [`Primary`] as the selection's highest-selection-priority entity,
/// ties broken by lowest [`SimulationId`], so a mixed selection leads with its
/// most significant unit (a caster over line infantry).
pub fn track_primary(
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    entities: Query<&EntityInfoComponent>,
    mut primary: ResMut<Primary>,
) {
    let local = session.local_player();
    let next = selection
        .get(local)
        .iter()
        .filter_map(|&id| {
            entities.iter().find(|info| info.id() == id).map(|info| {
                let priority = registry
                    .entity(info.type_name())
                    .map_or(0, |def| def.selection.priority());
                (priority, id)
            })
        })
        .max_by(|(pa, ia), (pb, ib)| pa.cmp(pb).then(ib.cmp(ia)))
        .map(|(_, id)| id);
    // Only touch the resource when it actually changes, so command-card rebuilds
    // (which key off change detection) fire on real selection changes, not every frame.
    primary.set_if_neq(Primary(next));
}

/// `F`/`R`/`G` arm a combat order for the current selection; the next
/// left-click supplies its target.
pub fn order_mode_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut mode: ResMut<InputMode>,
) {
    // Arm from Normal, or re-arm a different order; never steal an
    // in-progress building placement.
    if matches!(*mode, InputMode::PlacingBuild(_)) {
        return;
    }
    if selection.get(session.local_player()).is_empty() {
        return;
    }
    let armed = if keys.just_pressed(KeyCode::KeyF) {
        TargetedOrder::AttackMove
    } else if keys.just_pressed(KeyCode::KeyR) {
        TargetedOrder::Patrol
    } else if keys.just_pressed(KeyCode::KeyG) {
        TargetedOrder::Guard
    } else {
        return;
    };
    *mode = InputMode::Targeting(armed);
}

/// While a combat order is armed, left-click issues it (Esc/RMB cancel).
pub fn targeting_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    registry: Res<ContentRegistry>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
) {
    let InputMode::Targeting(armed) = *mode else {
        return;
    };

    if keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right) {
        *mode = InputMode::Normal;
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if pointer_over_ui(&interactions) {
        // A click on a HUD button belongs to the UI; keep the order armed so a
        // later click on the map still issues it.
        return;
    }
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };

    let flush = !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));
    match armed {
        TargetedOrder::AttackMove => {
            pending.push(PlayerCommand::AttackMove {
                target: world_to_pos(cursor),
                flush,
            });
        }
        TargetedOrder::Patrol => {
            pending.push(PlayerCommand::Patrol {
                target: world_to_pos(cursor),
                flush,
            });
        }
        TargetedOrder::Guard => {
            // Guard needs an entity under the cursor; a miss keeps the mode
            // armed so the player can click again.
            let Some(target) =
                world_to_cell(cursor).and_then(|cell| entity_at(cell, &registry, &entities))
            else {
                return;
            };
            pending.push(PlayerCommand::Guard { target, flush });
        }
    }
    *mode = InputMode::Normal;
}

/// `X` cycles the selection's stance, starting from the primary entity's.
pub fn stance_input(
    keys: Res<ButtonInput<KeyCode>>,
    primary: Res<Primary>,
    mut pending: ResMut<PendingInput>,
    stances: Query<(&EntityInfoComponent, &StanceComponent)>,
) {
    if !keys.just_pressed(KeyCode::KeyX) {
        return;
    }
    let Some(id) = primary.0 else {
        return;
    };
    let Some((_, StanceComponent(current))) = stances.iter().find(|(info, _)| info.id() == id)
    else {
        return;
    };
    let next = match current {
        Stance::Flee => Stance::HoldFire,
        Stance::HoldFire => Stance::StandGround,
        Stance::StandGround => Stance::Defend,
        Stance::Defend => Stance::Flee,
    };
    pending.push(PlayerCommand::SetStance { stance: next });
}

/// The last bare control-group recall (elapsed seconds, group), so a quick
/// second press of the same number also centers the camera on the group.
#[derive(Resource, Default)]
pub struct LastRecall(Option<(f32, u8)>);

/// Number keys 1–9,0 manage control groups: `Ctrl` assigns the current
/// selection, `Ctrl+Shift` appends to it, `Shift` recalls into the current
/// selection, and a bare press recalls (replacing) — a bare double-press also
/// snaps the camera to the group. Assign/append/recall are synced commands; the
/// camera move is local.
pub fn control_group_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    session: Res<GameSession>,
    groups: Res<ControlGroups>,
    mut last_recall: ResMut<LastRecall>,
    mut pending: ResMut<PendingInput>,
    map: Res<Map>,
    positions: Query<(&EntityInfoComponent, &LocationComponent)>,
    mut cameras: Query<&mut Transform, With<Camera2d>>,
) {
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ];
    let Some(index) = DIGITS.iter().position(|&key| keys.just_pressed(key)) else {
        return;
    };
    let group = index as u8;
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    if ctrl {
        pending.push(if shift {
            PlayerCommand::AppendGroup { group }
        } else {
            PlayerCommand::AssignGroup { group }
        });
        return;
    }

    let mode = if shift {
        SelectMode::Add
    } else {
        SelectMode::Replace
    };
    pending.push(PlayerCommand::RecallGroup { group, mode });

    if !shift {
        let now = time.elapsed_secs();
        let double = last_recall
            .0
            .is_some_and(|(t, g)| g == group && now - t <= DOUBLE_CLICK_SECS);
        last_recall.0 = Some((now, group));
        if double {
            center_on_group(index, &session, &groups, &map, &positions, &mut cameras);
        }
    }
}

/// Snaps the camera to the centroid of control group `group`'s living members.
fn center_on_group(
    group: usize,
    session: &GameSession,
    groups: &ControlGroups,
    map: &Map,
    positions: &Query<(&EntityInfoComponent, &LocationComponent)>,
    cameras: &mut Query<&mut Transform, With<Camera2d>>,
) {
    let ids = groups.get(session.local_player(), group);
    let mut sum = Vec2::ZERO;
    let mut count = 0.0;
    for &id in ids {
        if let Some((_, location)) = positions.iter().find(|(info, _)| info.id() == id) {
            sum.x += location.position.x.to_num::<f32>();
            sum.y += location.position.y.to_num::<f32>();
            count += 1.0;
        }
    }
    if count == 0.0 {
        return;
    }
    if let Ok(mut transform) = cameras.single_mut() {
        transform.translation.x = sum.x / count * CELL_PX;
        transform.translation.y = -sum.y / count * CELL_PX;
        // Keep the recenter within the map, matching pan/zoom.
        transform.translation = camera::clamp_to_map(transform.translation, map);
    }
}

/// While placing, draw a footprint ghost and place on left-click (Esc/RMB cancel).
pub fn placement_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
    mut gizmos: Gizmos,
    primary: Res<Primary>,
    map: Res<Map>,
    registry: Res<ContentRegistry>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let InputMode::PlacingBuild(type_name) = &*mode else {
        return;
    };
    let type_name = type_name.clone();

    if keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right) {
        *mode = InputMode::Normal;
        return;
    }
    // Don't ghost or place under the cursor while it is over a HUD button — in
    // particular, the build button's own click that just entered this mode.
    if pointer_over_ui(&interactions) {
        return;
    }

    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };
    let Some((cx, cy)) = world_to_cell(cursor) else {
        return;
    };
    let Some(location_def) = registry.entity(&type_name).and_then(|def| def.location) else {
        *mode = InputMode::Normal;
        return;
    };
    let size = location_def.size();

    let passable = map.nav_grid().is_footprint_passable_by(
        location_def.occupation(),
        NavPos::new(cx, cy),
        size,
    );
    let center = Vec2::new(
        (cx as f32 + size.width as f32 / 2.0) * CELL_PX,
        -(cy as f32 + size.height as f32 / 2.0) * CELL_PX,
    );
    let extent = Vec2::new(size.width as f32, size.height as f32) * CELL_PX;
    let color = if passable {
        Color::srgb(0.3, 1.0, 0.4)
    } else {
        Color::srgb(1.0, 0.3, 0.3)
    };
    gizmos.rect_2d(Isometry2d::from_translation(center), extent, color);

    if mouse.just_pressed(MouseButton::Left)
        && let Some(builder) = primary.0
    {
        pending.push(PlayerCommand::BuildEntity {
            builder,
            type_name,
            position: FixedUVec2::new(FixedU64::from_num(cx), FixedU64::from_num(cy)),
            flush: true,
        });
        *mode = InputMode::Normal;
    }
}
