//! Debug overlay: a live input/sim readout, gizmos, and sandbox spawn.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ferrets_bevy_plugin::PendingInput;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        entity_info::EntityInfoComponent, hidden::HiddenComponent, location::LocationComponent,
        order_queue::OrderQueueComponent, patrol::PatrolComponent,
    },
    content::registry::ContentRegistry,
    map::Map,
    order::Order,
    selection::Selection,
    session::GameSession,
    visibility::VisibilityGrid,
};

use crate::input::InputMode;
use crate::map;
use crate::render::{CELL_PX, FogReveal, world_center};
use crate::states::InGameUi;

/// Toggleable debug options.
#[derive(Resource)]
pub struct DebugState {
    /// Draw the debug overlay — nav grid and order lines.
    pub grid: bool,
    /// Type spawned.
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

/// Toggles the debug overlay.
pub fn toggle_debug(keys: Res<ButtonInput<KeyCode>>, mut debug: ResMut<DebugState>) {
    if keys.just_pressed(KeyCode::F1) {
        debug.grid = !debug.grid;
    }
}

/// Spawns the debug unit for the local player at the cursor cell, via the
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
    registry: Res<ContentRegistry>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
    mut text: Query<&mut Text, With<DebugText>>,
) {
    let cell = cursor_cell(&windows, &cameras);
    let cell_str = cell.map_or_else(|| "-".to_string(), |(x, y)| format!("({x},{y})"));

    // What the hit-test finds under the cursor (the same test selection uses).
    let hover = cell
        .and_then(|(cx, cy)| {
            entities.iter().find(|(info, location)| {
                let ox = location.position.x.to_num::<u32>();
                let oy = location.position.y.to_num::<u32>();
                let size = registry.def(info.type_id()).location.unwrap().size();
                cx >= ox && cx < ox + size.width && cy >= oy && cy < oy + size.height
            })
        })
        .map(|(info, _)| info.type_name().to_string());
    let hover_str = hover.as_deref().unwrap_or("-");

    let selected = selection.get(session.local_player()).len();
    let mode_str = match &*mode {
        InputMode::Normal => "normal",
        InputMode::PlacingBuild(_) => "placing",
        InputMode::Targeting(_) => "targeting",
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

/// Draws a faint grid over the playable area, tinting occupied ground cells
/// (run in `Update`), when enabled.
pub fn draw_grid(
    mut gizmos: Gizmos,
    map: Res<Map>,
    registry: Res<ContentRegistry>,
    debug: Res<DebugState>,
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
) {
    if !debug.grid {
        return;
    }
    let (w, h) = (map.width() as f32, map.height() as f32);
    let line = Color::srgba(0.0, 0.0, 0.0, 0.15);

    // Fill occupied cells so the nav grid's occupancy is visible at a glance —
    // but only where the local team can see, so fogged entities' footprints
    // don't leak their positions through the overlay.
    let local = session.local_player();
    let nav_grid = map.nav_grid();
    if let Some(ground) = registry.layer(map::GROUND) {
        for y in 0..map.height() {
            for x in 0..map.width() {
                if nav_grid.is_occupied(ground, NavPos::new(x, y))
                    && (reveal.0 || fog.is_visible_to(&session, local, x, y))
                {
                    fill_cell(&mut gizmos, x, y);
                }
            }
        }
    }

    for x in 0..=map.width() {
        let xp = x as f32 * CELL_PX;
        gizmos.line_2d(Vec2::new(xp, 0.0), Vec2::new(xp, -h * CELL_PX), line);
    }
    for y in 0..=map.height() {
        let yp = -(y as f32) * CELL_PX;
        gizmos.line_2d(Vec2::new(0.0, yp), Vec2::new(w * CELL_PX, yp), line);
    }
}

/// Draws every unit's order queue while the debug overlay is on: a line per
/// order from the unit to its target, colored by kind — moves green, combat
/// red, guard/follow cyan, harvest gold, build blue (run in `Update`).
pub fn draw_orders(
    mut gizmos: Gizmos,
    debug: Res<DebugState>,
    registry: Res<ContentRegistry>,
    units: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &OrderQueueComponent,
            Option<&PatrolComponent>,
            &Visibility,
        ),
        Without<HiddenComponent>,
    >,
    targets: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
) {
    const MOVE: Color = Color::srgb(0.3, 0.85, 0.4);
    const COMBAT: Color = Color::srgb(1.0, 0.35, 0.25);
    const GUARD: Color = Color::srgb(0.3, 0.9, 0.9);
    const HARVEST: Color = Color::srgb(0.85, 0.7, 0.2);
    const BUILD: Color = Color::srgb(0.35, 0.55, 1.0);

    if !debug.grid {
        return;
    }

    let cell_center = |position: FixedUVec2| {
        world_center(FixedUVec2::from(NavPos::from(position)), NavSize::ONE).truncate()
    };
    let entity_center = |id| {
        targets
            .iter()
            .find(|(info, ..)| info.id() == id)
            .map(|(info, location)| {
                let size = registry.def(info.type_id()).location.unwrap().size();
                world_center(location.position, size).truncate()
            })
    };

    for (info, location, queue, patrol, visibility) in &units {
        // Don't reveal a fogged unit's orders (its sprite is hidden by fog).
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let size = registry.def(info.type_id()).location.unwrap().size();
        let start = world_center(location.position, size).truncate();
        for entry in &queue.0 {
            let (end, color) = match &entry.order {
                Order::Move { target, .. } => (Some(cell_center(*target)), MOVE),
                Order::AttackMove { target } => (Some(cell_center(*target)), COMBAT),
                Order::Attack { target, .. } => (entity_center(*target), COMBAT),
                Order::Guard { target } => (entity_center(*target), GUARD),
                Order::Follow { target } => (entity_center(*target), GUARD),
                Order::Harvest { target } => (entity_center(*target), HARVEST),
                Order::Build { position, .. } => (Some(cell_center(*position)), BUILD),
                Order::Patrol { target } => {
                    // Both patrol endpoints; before the order starts, the
                    // return point is where the unit stands.
                    let end = cell_center(*target);
                    let home = cell_center(patrol.map_or(location.position, |p| p.home));
                    gizmos.line_2d(home, end, COMBAT);
                    gizmos.circle_2d(end, CELL_PX * 0.25, COMBAT);
                    gizmos.circle_2d(home, CELL_PX * 0.25, COMBAT);
                    continue;
                }
                Order::Train | Order::Die => continue,
            };
            // A vanished target leaves nothing to point at.
            let Some(end) = end else { continue };
            gizmos.line_2d(start, end, color);
            gizmos.circle_2d(end, CELL_PX * 0.25, color);
        }
    }
}

/// Tints a single cell red by stacking translucent lines (gizmos have no fill).
fn fill_cell(gizmos: &mut Gizmos, x: u32, y: u32) {
    const STEP_PX: f32 = 4.0;
    let fill = Color::srgba(1.0, 0.2, 0.2, 0.35);
    let left = x as f32 * CELL_PX;
    let right = left + CELL_PX;
    let top = -(y as f32) * CELL_PX;

    let mut offset = STEP_PX / 2.0;
    while offset < CELL_PX {
        let yp = top - offset;
        gizmos.line_2d(Vec2::new(left, yp), Vec2::new(right, yp), fill);
        offset += STEP_PX;
    }
}
