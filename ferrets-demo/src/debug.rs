//! Debug overlay: a live input/sim readout, gizmos, and sandbox spawn.

use bevy::{prelude::*, window::PrimaryWindow};
use ferrets_bevy_plugin::{PendingInput, TickPacing};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

use ferrets_content::{entity_stats::EntityStatId, registry::ContentRegistry};
use ferrets_physics::body;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        entity_info::EntityInfoComponent, entity_stats::StatsComponent, hidden::HiddenComponent,
        location::LocationComponent, movement::MoveComponent, order_queue::OrderQueueComponent,
        patrol::PatrolComponent,
    },
    map::Map,
    movement_model::MovementModel,
    order::{AttackTarget, Order},
    selection::Selection,
    session::GameSession,
    visibility::VisibilityGrid,
};

use crate::{
    input::InputMode,
    map,
    render::{self, CELL_PX, FogReveal, Smoothing, world_center},
    sound::Muted,
    states::InGameUi,
};

/// Toggleable debug options.
#[derive(Resource)]
pub struct DebugState {
    /// Draw the debug overlay — nav grid and order lines.
    pub grid: bool,
    /// Which navigation layer the occupancy fill shows, by registered name.
    /// Cycled with `F3`, because the layers hide each other: a flier's claim and
    /// the ground under it are different planes of the same cells.
    pub layer: String,
    /// Type spawned.
    pub spawn_type: String,
}

impl Default for DebugState {
    fn default() -> Self {
        Self {
            grid: true,
            layer: map::GROUND.into(),
            spawn_type: "archer".into(),
        }
    }
}

/// How many selected entities the readout names outright before it falls back to
/// counting them.
const NAMED_SELECTION: usize = 6;

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
            // Below the resource and supply lines (top 8 and 34).
            top: Val::Px(60.0),
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

/// Toggles the debug overlay, and cycles which navigation layer its occupancy
/// fill shows.
pub fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    registry: Res<ContentRegistry>,
    mut debug: ResMut<DebugState>,
) {
    if keys.just_pressed(KeyCode::F1) {
        debug.grid = !debug.grid;
    }
    if keys.just_pressed(KeyCode::F3) {
        // Registered layers in name order, so the cycle is stable across runs.
        let names: Vec<&str> = registry.layers().map(|(name, _)| name).collect();
        if let Some(next) = names
            .iter()
            .position(|&name| name == debug.layer)
            .map(|at| names[(at + 1) % names.len()])
            .or_else(|| names.first().copied())
        {
            debug.layer = next.to_string();
        }
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
    fixed: Res<Time<Fixed>>,
    pacing: Res<TickPacing>,
    selection: Res<Selection>,
    mode: Res<InputMode>,
    map: Res<Map>,
    registry: Res<ContentRegistry>,
    debug: Res<DebugState>,
    smoothing: Res<Smoothing>,
    muted: Res<Muted>,
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
        .map(|(info, _)| format!("{} #{}", info.type_name(), info.id().0));
    let hover_str = hover.as_deref().unwrap_or("-");

    // Named by simulation id rather than counted: a report about one unit of a
    // crowd has to be able to say which, and the cursor reaches what the
    // selection cannot — an enemy's, a neutral's, a seam's. Past a handful the
    // ids stop being readable and stop fitting, so the count is what the line
    // carries and hovering names the one that matters.
    let selected = session
        .local_player()
        .map_or(&[][..], |local| selection.get(local));
    let selection_str = match selected {
        [] => "-".to_string(),
        ids if ids.len() <= NAMED_SELECTION => ids
            .iter()
            .map(|id| format!("#{}", id.0))
            .collect::<Vec<_>>()
            .join(" "),
        ids => format!("{} units", ids.len()),
    };
    let mode_str = match &*mode {
        InputMode::Normal => "normal",
        InputMode::PlacingBuild(_) => "placing",
        InputMode::Targeting(_) => "targeting",
    };

    let model_str = match map.movement_model() {
        MovementModel::Cell => "cell",
        MovementModel::Continuous => "continuous",
    };

    // The cadence the game is asking for, and the one it is actually holding: a
    // throttled game is slower than its chosen speed, and this is where that
    // shows.
    // The timestep already carries the throttle, so it *is* the cadence being
    // held; what was asked for is that cadence divided back out.
    let held_hz = 1.0 / fixed.timestep().as_secs_f32();
    let wanted_hz = held_hz / pacing.throttle.to_num::<f32>();

    if let Ok(mut text) = text.single_mut() {
        **text = format!(
            "tick {} | {held_hz:.1}/{wanted_hz:.0} Hz | {} | {} | sound {} | layer {} | cursor {} | hover {} | LMB {} RMB {} | selected {} | {}",
            session.tick(),
            model_str,
            if smoothing.0 { "smoothed" } else { "per tick" },
            if muted.0 { "off" } else { "on" },
            debug.layer,
            cell_str,
            hover_str,
            mouse.pressed(MouseButton::Left) as u8,
            mouse.pressed(MouseButton::Right) as u8,
            selection_str,
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
    watch: Res<render::ObserverPerspective>,
    reveal: Res<FogReveal>,
) {
    if !debug.grid {
        return;
    }
    let (w, h) = (map.width() as f32, map.height() as f32);
    let line = Color::srgba(0.0, 0.0, 0.0, 0.15);

    // Fill occupied cells so the nav grid's occupancy is visible at a glance —
    // but only where this node can see, so fogged entities' footprints don't
    // leak their positions through the overlay.
    let nav_grid = map.nav_grid();
    if let Some(layer) = registry.layer(&debug.layer) {
        for y in 0..map.height() {
            for x in 0..map.width() {
                if nav_grid.is_occupied(layer, CellPos::new(x, y))
                    && (reveal.0 || render::sees(&session, &watch, &fog, x, y))
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

/// Draws each mover's collision circle and a line from its center to the
/// one cell it claims while the debug overlay is on and the game runs the
/// continuous model (run in `Update`). The circle is the contact truth, the
/// line points at the claim derived from it — together they make occupancy
/// checkable at a glance: bodies may touch and even share a claimed cell,
/// but circles must never interpenetrate. Fogged movers stay undrawn, like
/// the grid's occupancy fill.
pub fn draw_bodies(
    mut gizmos: Gizmos,
    debug: Res<DebugState>,
    map: Res<Map>,
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    watch: Res<render::ObserverPerspective>,
    reveal: Res<FogReveal>,
    registry: Res<ContentRegistry>,
    movers: Query<
        (&EntityInfoComponent, &LocationComponent, &StatsComponent),
        Without<HiddenComponent>,
    >,
) {
    const BODY: Color = Color::srgb(0.35, 0.9, 1.0);
    const CLAIM: Color = Color::srgba(0.35, 0.9, 1.0, 0.5);

    if !debug.grid {
        return;
    }
    match map.movement_model() {
        MovementModel::Cell => return,
        MovementModel::Continuous => {}
    }

    for (info, location, stats) in &movers {
        let def = registry.def(info.type_id());
        let claims = def
            .location
            .is_some_and(|location| location.solidity().claims_cells());
        if !def.can_move() || !claims {
            continue;
        }
        let cell = body::anchor(location.position);
        if !(reveal.0 || render::sees(&session, &watch, &fog, cell.x, cell.y)) {
            continue;
        }
        let Some(radius) = stats.effective(EntityStatId::RADIUS) else {
            continue;
        };

        // The circle sits half a footprint past the anchor, so a wider body is
        // drawn where it actually is rather than off its own near corner.
        let size = def
            .location
            .map(|location| location.size())
            .unwrap_or(CellSize::ONE);
        let center = Vec2::new(
            (location.position.x.to_num::<f32>() + size.width as f32 / 2.0) * CELL_PX,
            -(location.position.y.to_num::<f32>() + size.height as f32 / 2.0) * CELL_PX,
        );
        gizmos.circle_2d(center, radius.to_num::<f32>() * CELL_PX, BODY);

        let claimed = Vec2::new(
            (cell.x as f32 + 0.5) * CELL_PX,
            -(cell.y as f32 + 0.5) * CELL_PX,
        );
        gizmos.line_2d(center, claimed, CLAIM);
        gizmos.circle_2d(claimed, 2.0, CLAIM);
    }
}

/// Draws the pathfinding hierarchy while the debug overlay is on: cluster
/// borders, the entrances crossing them, and each selected unit's plan — its
/// refined cells and the corridor crossings still ahead (run in `Update`).
pub fn draw_hierarchy(
    mut gizmos: Gizmos,
    map: Res<Map>,
    registry: Res<ContentRegistry>,
    debug: Res<DebugState>,
    selection: Res<Selection>,
    session: Res<GameSession>,
    units: Query<(&EntityInfoComponent, &LocationComponent, &MoveComponent)>,
) {
    const CLUSTER: Color = Color::srgba(0.9, 0.7, 0.1, 0.5);
    const ENTRANCE: Color = Color::srgba(0.9, 0.7, 0.1, 0.9);
    const SEGMENT: Color = Color::srgb(0.2, 0.9, 0.9);
    const CORRIDOR: Color = Color::srgba(0.2, 0.9, 0.9, 0.5);

    if !debug.grid {
        return;
    }
    let hierarchy = map.hierarchy();
    let (w, h) = (map.width() as f32, map.height() as f32);

    // Cluster borders.
    let size = hierarchy.cluster_size();
    for x in (size..map.width()).step_by(size as usize) {
        let xp = x as f32 * CELL_PX;
        gizmos.line_2d(Vec2::new(xp, 0.0), Vec2::new(xp, -h * CELL_PX), CLUSTER);
    }
    for y in (size..map.height()).step_by(size as usize) {
        let yp = -(y as f32) * CELL_PX;
        gizmos.line_2d(Vec2::new(0.0, yp), Vec2::new(w * CELL_PX, yp), CLUSTER);
    }

    // Entrances of every mover shape the hierarchy serves. A wide mover has its
    // own, narrower set, so drawing all of them shows where clearance bites.
    let cell_center =
        |cell: CellPos| world_center(FixedUVec2::from(cell), CellSize::ONE).truncate();
    for shape in hierarchy.shapes() {
        for transition in hierarchy.transitions(shape) {
            gizmos.line_2d(
                cell_center(transition.a),
                cell_center(transition.b),
                ENTRANCE,
            );
            gizmos.circle_2d(cell_center(transition.a), CELL_PX * 0.15, ENTRANCE);
        }
    }

    // Each selected unit's plan: the refined segment cell by cell, then the
    // corridor crossings still ahead.
    let selected = session
        .local_player()
        .map_or(&[][..], |local| selection.get(local));
    for (info, location, movement) in &units {
        if !selected.contains(&info.id()) {
            continue;
        }
        let size = registry.def(info.type_id()).location.unwrap().size();
        let mut from = world_center(location.position, size).truncate();
        for cell in movement.path.iter().rev() {
            let to = cell_center(*cell);
            gizmos.line_2d(from, to, SEGMENT);
            from = to;
        }
        for crossing in movement.corridor.iter().rev() {
            let to = cell_center(crossing.from);
            gizmos.line_2d(from, to, CORRIDOR);
            gizmos.circle_2d(to, CELL_PX * 0.2, CORRIDOR);
            from = cell_center(crossing.to);
        }
    }
}

/// Draws every unit's order queue while the debug overlay is on: a line per
/// order from the unit to its target, colored by the order's kind (run in
/// `Update`).
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
    // Matches the always-on work-link hue, and stays apart from MOVE's green.
    const REPAIR: Color = Color::srgb(0.9, 0.5, 0.9);

    if !debug.grid {
        return;
    }

    // A destination is named by the first cell of its footprint, so a line drawn to
    // the position alone points at a corner of what the unit is walking to.
    let footprint_center = |position: FixedUVec2, size: CellSize| {
        world_center(FixedUVec2::from(CellPos::from(position)), size).truncate()
    };
    let cell_center = |position: FixedUVec2| footprint_center(position, CellSize::ONE);
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
                Order::Move { target, size, .. } => (Some(footprint_center(*target, *size)), MOVE),
                Order::AttackMove { target } => (Some(cell_center(*target)), COMBAT),
                Order::Attack { target, .. } => match target {
                    AttackTarget::Entity(id) => (entity_center(*id), COMBAT),
                    AttackTarget::Position(cell) => (Some(cell_center(*cell)), COMBAT),
                },
                Order::Guard { target } => (entity_center(*target), GUARD),
                Order::Follow { target } => (entity_center(*target), GUARD),
                Order::Harvest { target } => (entity_center(*target), HARVEST),
                Order::Build {
                    type_name,
                    position,
                } => {
                    let size = registry
                        .entity(type_name)
                        .and_then(|def| def.location)
                        .map_or(CellSize::ONE, |location| location.size());
                    (Some(footprint_center(*position, size)), BUILD)
                }
                Order::Repair { target } => (entity_center(*target), REPAIR),
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
                Order::Board { target } => (entity_center(*target), GUARD),
                Order::Load { target } => (entity_center(*target), GUARD),
                Order::Unload { at } => match at {
                    Some(position) => (Some(cell_center(*position)), GUARD),
                    None => continue,
                },
                // A form change happens where the unit stands, so it has no line.
                Order::Train | Order::Research { .. } | Order::Morph { .. } | Order::Die => {
                    continue;
                }
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
