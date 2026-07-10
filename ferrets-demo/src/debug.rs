//! Debug overlay: a live input/sim readout, nav-grid toggle, and sandbox spawn.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ferrets_bevy_plugin::PendingInput;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        location::{LocationComponent, LocationStaticData},
    },
    selection::Selection,
    session::GameSession,
};

use crate::input::InputMode;
use crate::render::CELL_PX;
use crate::states::InGameUi;

/// Toggleable debug options.
#[derive(Resource)]
pub struct DebugState {
    /// Draw the nav grid (F1).
    pub grid: bool,
    /// Type spawned by F2.
    pub spawn_type: String,
}

impl Default for DebugState {
    fn default() -> Self {
        Self {
            grid: true,
            spawn_type: "archer".into(),
        }
    }
}

#[derive(Component)]
pub struct DebugText;

/// Spawns the debug readout line.
pub fn setup_debug(mut commands: Commands) {
    commands.spawn((
        InGameUi,
        DebugText,
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(0.6, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
}

/// The grid cell under the cursor, if the cursor is over the playable area.
fn cursor_cell(
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) -> Option<(u32, u32)> {
    let (window, (camera, camera_transform)) = (windows.single().ok()?, cameras.single().ok()?);
    let cursor = window.cursor_position()?;
    let world = camera.viewport_to_world_2d(camera_transform, cursor).ok()?;
    let (x, y) = (world.x / CELL_PX, -world.y / CELL_PX);
    (x >= 0.0 && y >= 0.0).then_some((x as u32, y as u32))
}

/// F1 toggles the grid.
pub fn toggle_debug(keys: Res<ButtonInput<KeyCode>>, mut debug: ResMut<DebugState>) {
    if keys.just_pressed(KeyCode::F1) {
        debug.grid = !debug.grid;
    }
}

/// F2 spawns the debug unit for the local player at the cursor cell, via the
/// `Spawn` command (deterministic command pipeline).
pub fn spawn_debug(
    keys: Res<ButtonInput<KeyCode>>,
    debug: Res<DebugState>,
    mut pending: ResMut<PendingInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    if !keys.just_pressed(KeyCode::F2) {
        return;
    }
    if let Some((x, y)) = cursor_cell(&windows, &cameras) {
        pending.push(PlayerCommand::Spawn {
            type_name: debug.spawn_type.clone(),
            position: FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y)),
        });
    }
}

/// Updates the diagnostic readout (cursor cell, hovered entity, mouse, selection, etc).
pub fn debug_readout(
    mouse: Res<ButtonInput<MouseButton>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mode: Res<InputMode>,
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
    mut text: Query<&mut Text, With<DebugText>>,
) {
    let cell = cursor_cell(&windows, &cameras);
    let cell_str = cell.map_or_else(|| "-".to_string(), |(x, y)| format!("({x},{y})"));

    // What the hit-test finds under the cursor (the same test selection uses).
    let hover = cell
        .and_then(|(cx, cy)| {
            entities.iter().find(|(_, location, location_data)| {
                let ox = location.position.x.to_num::<u32>();
                let oy = location.position.y.to_num::<u32>();
                let size = location_data.size();
                cx >= ox && cx < ox + size.width && cy >= oy && cy < oy + size.height
            })
        })
        .map(|(info, _, _)| info.type_name().to_string());
    let hover_str = hover.as_deref().unwrap_or("-");

    let selected = selection.get(session.local_player()).len();
    let mode_str = match &*mode {
        InputMode::Normal => "normal",
        InputMode::PlacingBuild(_) => "placing",
    };

    if let Ok(mut text) = text.single_mut() {
        **text = format!(
            "tick {} | cursor {} | hover {} | LMB {} RMB {} | selected {} | {}",
            session.tick(),
            cell_str,
            hover_str,
            mouse.pressed(MouseButton::Left) as u8,
            mouse.pressed(MouseButton::Right) as u8,
            selected,
            mode_str,
        );
    }
}
