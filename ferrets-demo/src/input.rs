//! Mouse input → simulation commands: left-click/drag selection, right-click
//! move/send orders. Everything is issued as a `PlayerCommand` through
//! `PendingInput`, so it flows through the deterministic command pipeline.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ferrets_bevy::{NetworkActive, PauseIntent, PendingInput};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::entity_info::EntityInfoComponent,
    components::{
        build::BuilderStaticData,
        hidden::HiddenComponent,
        location::{LocationComponent, LocationStaticData},
        train::TrainStaticData,
    },
    content::registry::ContentRegistry,
    map::Map,
    selection::Selection,
    session::GameSession,
    simulation_id::SimulationId,
};

use crate::map::GROUND;
use crate::render::CELL_PX;

/// Drag below this many pixels is treated as a click, not a box-select.
const CLICK_SLOP: f32 = 4.0;

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
}

/// World-space anchor of an in-progress left-drag.
#[derive(Resource, Default)]
pub struct DragStart(Option<Vec2>);

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

/// The selectable entity whose footprint covers `cell`, preferring the smallest
/// footprint (a unit standing on/near a building wins over the building).
fn entity_at(
    cell: (u32, u32),
    entities: &Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
        ),
        Without<HiddenComponent>,
    >,
) -> Option<SimulationId> {
    let mut best: Option<(u32, SimulationId)> = None;
    for (info, location, location_data) in entities {
        let ox = location.position.x.to_num::<u32>();
        let oy = location.position.y.to_num::<u32>();
        let size = location_data.size();
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
    mut drag: ResMut<DragStart>,
    mut gizmos: Gizmos,
    mut pending: ResMut<PendingInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
        ),
        Without<HiddenComponent>,
    >,
) {
    if !matches!(*mode, InputMode::Normal) {
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
        if start.distance(cursor) <= CLICK_SLOP
            && let Some(cell) = world_to_cell(cursor)
            && let Some(id) = entity_at(cell, &entities)
        {
            pending.push(PlayerCommand::SelectById { id });
        } else {
            pending.push(PlayerCommand::SelectByRect {
                rect: cell_rect(start, cursor),
            });
        }
    }
}

/// Right click sends the selection to the entity under the cursor, or moves it
/// to the clicked cell. Holding Shift appends instead of replacing orders.
pub fn order_input(
    mode: Res<InputMode>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut pending: ResMut<PendingInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
        ),
        Without<HiddenComponent>,
    >,
) {
    if !mouse.just_pressed(MouseButton::Right) || !matches!(*mode, InputMode::Normal) {
        return;
    }
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };

    let flush = !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));

    let target = world_to_cell(cursor).and_then(|cell| entity_at(cell, &entities));
    match target {
        Some(target) => pending.push(PlayerCommand::SendToEntity { target, flush }),
        None => pending.push(PlayerCommand::Move {
            target: world_to_pos(cursor),
            flush,
        }),
    }
}

/// The first selected entity's [`SimulationId`], if any.
fn primary(selection: &Selection, session: &GameSession) -> Option<SimulationId> {
    selection.get(session.local_player()).first().copied()
}

/// Number keys 1–4 queue a unit on the selected producer building.
pub fn train_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut pending: ResMut<PendingInput>,
    producers: Query<(&EntityInfoComponent, &TrainStaticData)>,
) {
    let Some(id) = primary(&selection, &session) else {
        return;
    };
    let Some((_, trainer)) = producers.iter().find(|(info, _)| info.id() == id) else {
        return;
    };
    const DIGITS: [KeyCode; 4] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ];
    for (i, name) in trainer.trains().take(DIGITS.len()).enumerate() {
        if keys.just_pressed(DIGITS[i]) {
            pending.push(PlayerCommand::TrainEntity {
                trainer: id,
                type_name: name.to_string(),
            });
        }
    }
}

/// `B` enters/cycles build-placement mode for the selected builder's catalogue.
pub fn build_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut mode: ResMut<InputMode>,
    builders: Query<(&EntityInfoComponent, &BuilderStaticData)>,
) {
    if !keys.just_pressed(KeyCode::KeyB) {
        return;
    }
    let Some(id) = primary(&selection, &session) else {
        return;
    };
    let Some((_, builder)) = builders.iter().find(|(info, _)| info.id() == id) else {
        return;
    };
    let builds: Vec<String> = builder.builds().map(String::from).collect();
    if builds.is_empty() {
        return;
    }
    let next = match &*mode {
        InputMode::PlacingBuild(current) => {
            let idx = builds
                .iter()
                .position(|b| b == current)
                .map_or(0, |i| (i + 1) % builds.len());
            builds[idx].clone()
        }
        InputMode::Normal => builds[0].clone(),
    };
    *mode = InputMode::PlacingBuild(next);
}

/// While placing, draw a footprint ghost and place on left-click (Esc/RMB cancel).
pub fn placement_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    selection: Res<Selection>,
    map: Res<Map>,
    registry: Res<ContentRegistry>,
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

    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };
    let Some((cx, cy)) = world_to_cell(cursor) else {
        return;
    };
    let Some(size) = registry
        .entity(&type_name)
        .and_then(|def| def.location)
        .map(|loc| loc.size())
    else {
        *mode = InputMode::Normal;
        return;
    };

    let passable = map
        .nav_grid()
        .is_footprint_passable_by(GROUND, NavPos::new(cx, cy), size);
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
        && let Some(builder) = primary(&selection, &session)
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
