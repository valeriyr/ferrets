//! Mouse input → simulation commands: left-click/drag selection, right-click
//! move/send orders. Everything is issued as a `PlayerCommand` through
//! `PendingInput`, so it flows through the deterministic command pipeline.

use bevy::{prelude::*, window::PrimaryWindow};
use ferrets_bevy_plugin::{NetworkActive, PauseIntent, PendingInput, SpeedIntent, tick};
use ferrets_content::{
    build::BuilderAttendance,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::FieldPlacement,
    location::Solidity,
    registry::ContentRegistry,
    skills::{EntityCastEffect, EntityCastTarget, SkillCaster, SkillId, SummonAt, SummonPlacement},
    work::WorkPresence,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::{layer_mask::LayerMask, mover_shape::MoverShape};
use ferrets_physics::body;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{
        attached::AttachedComponent,
        dying::RemainsComponent,
        entity_info::EntityInfoComponent,
        field_source::{Emitted, FieldSourcesComponent},
        location::LocationComponent,
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        rise::RisingComponent,
        stance::{Stance, StanceComponent},
        warp::WarpComponent,
    },
    control_groups::ControlGroups,
    entity_def,
    entity_index::EntityIndex,
    fields::{self, FieldGrid},
    game_loop::{
        cast::{self, AimRefusal},
        warp,
    },
    ground,
    input::SYNC_LATENCY,
    landing, make_way,
    map::Map,
    occupancy::OccupancyClass,
    order::AttackTarget,
    selection::Selection,
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
    visibility::{CellVisibility, VisibilityGrid},
};

use crate::{
    camera,
    render::{self, CELL_PX, FogReveal},
    time::{self, SpeedStep},
};

/// Drag below this many pixels is treated as a click, not a box-select.
const CLICK_SLOP: f32 = 4.0;

/// Window within which a repeat of the same action counts as a double input: a
/// second click on the same entity (select all of type) or a second press of the
/// same control-group key (recenter on the group).
const DOUBLE_CLICK_SECS: f32 = 0.35;

/// The last left-click on an entity (elapsed seconds, id), for double-click detection.
#[derive(Resource, Default)]
pub struct LastClick(Option<(f32, SimulationId)>);

/// Toggles pause on the `P` key by recording the local intent; the engine
/// applies it — immediately off the network, or as a tick-aligned change every
/// node applies together through the session's authority on it. The demo never
/// touches the session itself, so it cannot pick the wrong mechanism.
///
/// Steering a replay counts as watching, not commanding, so this stays live
/// during playback — pausing a recording is how a moment gets inspected.
pub fn pause_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    mut intent: ResMut<PauseIntent>,
) {
    if !keys.just_pressed(KeyCode::KeyP) {
        return;
    }
    intent.0 = Some(!session.is_paused());
}

/// Steps the game speed on the `-` and `=` keys (numpad too) by recording the
/// local intent; the engine applies it — immediately off the network, or as a
/// tick-aligned change every node applies together through the session's
/// authority on it. Fast-forward steps are refused while networked — nobody
/// else in the match agreed to be rushed.
///
/// The rung the keys step from is read off the session, the one owner of the
/// game's speed: a peer's change lands there without this node pressing
/// anything, and a networked request shows only once the authority applies it.
pub fn speed_input(
    keys: Res<ButtonInput<KeyCode>>,
    networked: Option<Res<NetworkActive>>,
    session: Res<GameSession>,
    mut intent: ResMut<SpeedIntent>,
) {
    let up = keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd);
    let down = keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract);
    let current = SpeedStep::of(session.speed());
    let wanted = match (up, down) {
        (true, false) => current.faster(),
        (false, true) => current.slower(),
        (true, true) | (false, false) => return,
    };
    // Fast-forward is refused while networked — nobody else agreed to be rushed —
    // but only as a step *into* it. Stepping down from a fast-forward rung (one a
    // peer put the session on) must stay possible, or the game cannot be slowed
    // back to a playable pace.
    let rushing = wanted.fast_forward() && !current.fast_forward();
    if wanted == current || (rushing && networked.is_some()) {
        return;
    }
    intent.0 = Some(wanted.speed());
}

/// Requests a single-tick step on the `.` key, for walking a paused moment
/// through tick by tick. Applied by the engine ahead of the next frame, which
/// also refuses it where it must not run — a networked session, a finished
/// replay.
pub fn step_input(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    if !keys.just_pressed(KeyCode::Period) {
        return;
    }
    commands.insert_resource(tick::Step);
}

/// Fast-forwards ten seconds on the `]` key — a minute with shift — through a
/// replay's recording or ahead in a local game. Applied by the engine ahead of
/// the next frame, which also refuses it where it must not run — a networked
/// session, a finished replay. A press while a seek is already running
/// retargets it from wherever it has reached, extending the fast-forward.
pub fn seek_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    mut commands: Commands,
) {
    if !keys.just_pressed(KeyCode::BracketRight) {
        return;
    }
    let seconds = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        60
    } else {
        10
    };
    let ticks = time::ticks(seconds);
    commands.insert_resource(tick::Seek(session.tick() + ticks));
}

/// What the next left-click does.
#[derive(Resource, Default)]
pub enum InputMode {
    /// Left-click selects.
    #[default]
    Normal,
    /// Left-click places the named building (started from the build menu).
    PlacingBuild(String),
    /// Left-click warps the named unit in by the first of the leading
    /// warper's candidates that may (started from its card).
    PlacingWarp(String),
    /// Left-click issues the armed combat order (started from a hotkey).
    Targeting(TargetedOrder),
}

/// A combat order armed by hotkey, waiting for its target click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetedOrder {
    /// `A` — attack the clicked entity, whoever owns it, or attack-move to the
    /// clicked cell when nothing stands under the cursor.
    Attack,
    /// `R` — patrol between here and the clicked position.
    Patrol,
    /// `G` — guard the clicked entity.
    Guard,
    /// `T` — follow the clicked entity, explicitly: the smart right-click
    /// reads a click on a friendly transporter as boarding, so tailing one
    /// needs its own verb.
    Follow,
    /// `B` — board the clicked transporter, explicitly: the smart right-click
    /// reads a click on a damaged one as repairing, so climbing into it needs
    /// its own verb.
    Board,
    /// Armed from the command card — the leading transporter fetches the
    /// clicked unit aboard.
    Load,
    /// Armed from the command card — unload the leading transporter at the
    /// clicked position.
    Unload,
    /// `Q` — shell the clicked cell. Only weapons that send their shots to a cell
    /// take the order; the rest ignore it.
    AttackGround,
    /// Armed from the command card — cast the skill on the clicked entity.
    /// Selected units that lack the skill, and invalid targets, ignore the cast.
    Skill(SkillId),
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

/// Cell-space position under a world point (clamped to the non-negative quadrant).
fn world_to_pos(world: Vec2) -> FixedUVec2 {
    FixedUVec2::new(
        FixedU64::from_num((world.x / CELL_PX).max(0.0)),
        FixedU64::from_num((-world.y / CELL_PX).max(0.0)),
    )
}

/// The world points under the four corners of the screen frame spanned by
/// `anchor` (a world point) and the cursor. The frame is a screen-aligned
/// rectangle; under a turned camera its corners land on four distinct world
/// points, so all four matter.
fn frame_corners(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    anchor: Vec2,
    cursor: Vec2,
) -> Option<[Vec2; 4]> {
    let a = camera
        .world_to_viewport(camera_transform, anchor.extend(0.0))
        .ok()?;
    let b = camera
        .world_to_viewport(camera_transform, cursor.extend(0.0))
        .ok()?;
    let min = a.min(b);
    let max = a.max(b);
    let corner = |v| camera.viewport_to_world_2d(camera_transform, v).ok();
    Some([
        corner(min)?,
        corner(Vec2::new(max.x, min.y))?,
        corner(max)?,
        corner(Vec2::new(min.x, max.y))?,
    ])
}

/// The cell-space bounding box of world `points` — what a screen frame or
/// the viewport covers, as the rectangle the selection commands carry.
fn covering_rect(points: &[Vec2]) -> FixedURect {
    let min = points.iter().copied().reduce(Vec2::min).unwrap();
    let max = points.iter().copied().reduce(Vec2::max).unwrap();
    let low = Vec2::new(min.x / CELL_PX, -max.y / CELL_PX).max(Vec2::ZERO);
    let high = Vec2::new(max.x / CELL_PX, -min.y / CELL_PX).max(Vec2::ZERO);
    FixedURect::from_corners(
        FixedUVec2::new(FixedU64::from_num(low.x), FixedU64::from_num(low.y)),
        FixedUVec2::new(FixedU64::from_num(high.x), FixedU64::from_num(high.y)),
    )
}

/// The rectangle currently visible in the viewport, for on-screen
/// select-all-of-type (a double-click grabs only what's on screen).
fn visible_rect(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> FixedURect {
    covering_rect(&viewport_corners(window, camera, camera_transform))
}

/// The world points under the four corners of the viewport, clockwise from its
/// top-left. Under a turned camera these are four distinct points that do not
/// form an axis-aligned rectangle, so consumers that care about the shape of
/// the visible region must keep all four.
pub fn viewport_corners(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> [Vec2; 4] {
    let size = Vec2::new(window.width(), window.height());
    let corner = |v| {
        camera
            .viewport_to_world_2d(camera_transform, v)
            .unwrap_or(Vec2::ZERO)
    };
    [
        corner(Vec2::ZERO),
        corner(Vec2::new(size.x, 0.0)),
        corner(size),
        corner(Vec2::new(0.0, size.y)),
    ]
}

/// What a click is looking for under the cursor.
#[derive(Clone, Copy)]
pub(crate) enum Clicked {
    /// Whatever is standing there — what every order means.
    Standing,
    /// The body lying there, which only a cast that spends one means.
    Remains,
}

/// The selectable entity whose sprite covers the world position, preferring
/// the smallest footprint (a unit standing on/near a building wins over the
/// building). Sprites span the entity's fractional position plus its
/// footprint — the same box they are rendered in — so a mover resting
/// between cell origins is picked by what is drawn, not by the cell its
/// position truncates to.
pub(crate) fn entity_at(
    world: Vec2,
    looking_for: Clicked,
    registry: &ContentRegistry,
    entities: &Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
) -> Option<SimulationId> {
    let x = world.x / CELL_PX;
    let y = -world.y / CELL_PX;
    let mut best: Option<(u32, SimulationId)> = None;
    for (info, location, visibility, sighted, remains) in entities {
        // The click meets what the player sees: a sprite the fog (or the
        // watching perspective) hides is not there to be clicked, and the
        // simulation would refuse an order against it anyway. A glimpse is a
        // shimmer, not a thing: nothing to click either.
        if matches!(visibility, Some(Visibility::Hidden)) || !render::stamped(sighted).is_seen() {
            continue;
        }
        // A body lies in the same cell as whoever is standing over it, so
        // which of the two a click means is the click's own business: only a
        // cast that spends a body ever means the body.
        let wanted = match looking_for {
            Clicked::Standing => !remains,
            Clicked::Remains => remains,
        };
        if !wanted {
            continue;
        }
        let ox = location.position.x.to_num::<f32>();
        let oy = location.position.y.to_num::<f32>();
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        // An airborne sprite is drawn lifted up the screen; the click meets
        // the unit where the player sees it, so the lift comes back off the
        // cursor before the footprint test.
        let y = y + render::lift(registry, def).y / CELL_PX;
        let inside =
            x >= ox && x < ox + size.width as f32 && y >= oy && y < oy + size.height as f32;
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
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
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
        && let Some(corners) = frame_corners(camera, camera_transform, start, cursor)
    {
        // Live selection box: a screen-aligned frame, drawn through its
        // world corners so it stays screen-square under any camera look.
        let outline = [corners[0], corners[1], corners[2], corners[3], corners[0]];
        gizmos.linestrip_2d(outline, Color::srgb(0.2, 1.0, 0.4));
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
            && let Some(id) = entity_at(cursor, Clicked::Standing, &registry, &entities)
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
                    rect: visible_rect(window, camera, camera_transform),
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
            let rect = frame_corners(camera, camera_transform, start, cursor)
                .map(|corners| covering_rect(&corners))
                .unwrap_or_else(|| covering_rect(&[start, cursor]));
            pending.push(PlayerCommand::SelectByRect { rect, mode });
        }
    }
}

/// Right click sends the selection to the entity under the cursor, or moves it
/// to the clicked cell. Holding Shift appends instead of replacing orders.
/// When nothing selected can move yet, the click re-targets the rally points
/// of the own stationary producers and rising movers among the selection
/// instead — whatever else stands still beside them, a larva say, takes
/// neither an order nor a rally — and clicking anything in the selection
/// clears them. Anything that can move outranks rally: a risen mobile
/// rally-holder answers a right click by moving.
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
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
    holders: Query<(
        &EntityInfoComponent,
        &OwnerComponent,
        Option<&RallyPointComponent>,
        Has<RisingComponent>,
    )>,
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
    let target = entity_at(cursor, Clicked::Standing, &registry, &entities);
    issue_orders_at(
        cursor,
        target,
        flush,
        &session,
        &selection,
        &registry,
        &holders,
        &mut pending,
    );
}

/// Issues the orders a right click on `world` asks for: re-targeting the rally
/// points of the own stationary producers and rising movers among the
/// selection when nothing selected can move yet, sending the selection to
/// `target` when the click named one, and moving it to the point otherwise.
///
/// Shared by the cursor and the minimap, so a click means the same thing
/// wherever it lands. A caller with no way to aim at an entity passes no
/// `target` and always gets the positional reading.
pub fn issue_orders_at(
    world: Vec2,
    target: Option<SimulationId>,
    flush: bool,
    session: &GameSession,
    selection: &Selection,
    registry: &ContentRegistry,
    holders: &Query<(
        &EntityInfoComponent,
        &OwnerComponent,
        Option<&RallyPointComponent>,
        Has<RisingComponent>,
    )>,
    pending: &mut PendingInput,
) {
    // The click sets rally points only when nothing selected can move — a
    // mixed selection keeps ordering its units around normally, movement
    // outranking rally — and then only on the own entities in it that hold a
    // rally point: stationary producers, and movers still rising, which move
    // only once they have risen. A stationary thing with no rally point takes
    // nothing either way.
    let Some(local) = session.local_player() else {
        return;
    };
    let selected = selection.get(local);
    let mut rally_holders = Vec::new();
    let mut any_moves = false;
    for &id in selected {
        let Some((info, owner, rally, rising)) = holders.iter().find(|(info, ..)| info.id() == id)
        else {
            continue;
        };
        if registry.def(info.type_id()).can_move() && !rising {
            any_moves = true;
        } else if rally.is_some() && owner.player() == local {
            rally_holders.push(id);
        }
    }
    if !any_moves && !rally_holders.is_empty() {
        let target = match target {
            Some(id) if selected.contains(&id) => None,
            Some(id) => Some(RallyTarget::Entity(id)),
            None => Some(RallyTarget::Position(world_to_pos(world))),
        };
        for holder in rally_holders {
            pending.push(PlayerCommand::SetRallyPoint {
                entity: holder,
                target,
            });
        }
        return;
    }

    match target {
        Some(target) => pending.push(PlayerCommand::SendToEntity { target, flush }),
        None => pending.push(PlayerCommand::Move {
            target: world_to_pos(world),
            flush,
        }),
    }
}

/// The units that would clear ground a building needs for the local player,
/// in id order.
#[derive(Resource, Default)]
pub struct Yielders(pub Vec<SimulationId>);

/// Notes, while a building is being placed, which units would clear its
/// ground for the local player (see [`make_way::clears`]).
pub fn note_yielding(world: &mut World) {
    let ids: Vec<SimulationId> = match *world.resource::<InputMode>() {
        InputMode::PlacingBuild(_) => {
            let local = world.resource::<GameSession>().local_player();
            world
                .resource::<EntityIndex>()
                .alive_iter()
                .filter(|&(_, &entity)| make_way::clears(world, entity, local))
                .map(|(&id, _)| id)
                .collect()
        }
        InputMode::Normal | InputMode::PlacingWarp(_) | InputMode::Targeting(_) => Vec::new(),
    };
    world.resource_mut::<Yielders>().0 = ids;
}

/// Whether the placement ghost reads the cells of the entity `id`, of type
/// `type_id`, as free: the leading builder, which gets out of its own site's
/// way, and units that clear the ground for the side.
pub fn ghost_ignores(
    registry: &ContentRegistry,
    leading: Option<SimulationId>,
    yielders: &Yielders,
    id: SimulationId,
    type_id: EntityTypeId,
) -> bool {
    (Some(id) == leading && gets_out_of_way(registry.def(type_id)))
        || yielders.0.binary_search(&id).is_ok()
}

/// The local player's leading selected entity — the one hotkeys and panels act on.
#[derive(Resource, Default, PartialEq, Eq)]
pub struct Leading(pub Option<SimulationId>);

/// The entities a spectator selected to look at: display-only, local to this
/// node, and touching no synced state — a spectator's clicks never become
/// commands.
#[derive(Resource, Default)]
pub struct Inspected(pub Vec<SimulationId>);

/// Whether the local player is still at the controls: command-producing input
/// runs only while the game answers this node's commands. Its complement is
/// what the demo calls spectating — the session only certifies who plays;
/// that the human behind this screen is then *watching* is the demo's own
/// knowledge, since the demo is the screen.
pub fn commands_live(session: Res<GameSession>) -> bool {
    session.local_plays()
}

/// A spectator's selection: a left click picks the entity under the cursor
/// for the info panel (Shift toggles it in and out), a drag frames a box and
/// takes everything inside (Shift adds), a click on empty ground clears. All
/// of it local and display-only — no command is ever produced; the sim's
/// selection rules (buildings excluded from boxes, ownership) don't apply,
/// because a watcher inspects anything.
pub fn inspect_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    registry: Res<ContentRegistry>,
    mut drag: ResMut<DragStart>,
    mut inspected: ResMut<Inspected>,
    mut gizmos: Gizmos,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
) {
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
        && let Some(corners) = frame_corners(camera, camera_transform, start, cursor)
    {
        // The live box, drawn like the playing marquee so watching feels the
        // same as playing — only the outcome differs.
        let outline = [corners[0], corners[1], corners[2], corners[3], corners[0]];
        gizmos.linestrip_2d(outline, Color::srgb(0.85, 0.9, 1.0));
    }

    if mouse.just_released(MouseButton::Left)
        && let Some(start) = drag.0.take()
    {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if start.distance(cursor) <= CLICK_SLOP {
            match entity_at(cursor, Clicked::Standing, &registry, &entities) {
                Some(id) if shift => {
                    if let Some(held) = inspected.0.iter().position(|&it| it == id) {
                        inspected.0.remove(held);
                    } else {
                        inspected.0.push(id);
                    }
                }
                Some(id) => inspected.0 = vec![id],
                None if shift => {}
                None => inspected.0.clear(),
            }
        } else {
            let rect = frame_corners(camera, camera_transform, start, cursor)
                .map(|corners| covering_rect(&corners))
                .unwrap_or_else(|| covering_rect(&[start, cursor]));
            let framed = entities
                .iter()
                .filter(|(_, location, visibility, sighted, remains)| {
                    !matches!(visibility, Some(Visibility::Hidden))
                        && render::stamped(*sighted).is_seen()
                        && !remains
                        && rect.contains(location.position)
                })
                .map(|(info, ..)| info.id());
            if shift {
                for id in framed {
                    if !inspected.0.contains(&id) {
                        inspected.0.push(id);
                    }
                }
            } else {
                inspected.0 = framed.collect();
            }
        }
    }
}

/// Recomputes [`Leading`] as the selection's highest-selection-priority entity,
/// ties broken by lowest [`SimulationId`], so a mixed selection leads with its
/// most significant unit (a caster over line infantry).
pub fn track_leading(
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    entities: Query<&EntityInfoComponent>,
    mut leading: ResMut<Leading>,
) {
    let Some(local) = session.local_player() else {
        return;
    };
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
    leading.set_if_neq(Leading(next));
}

/// `A`/`R`/`G`/`T`/`B`/`Q` arm an order for the current selection; the next
/// left-click supplies its target.
pub fn order_mode_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    mut mode: ResMut<InputMode>,
) {
    // Arm from Normal, or re-arm a different order; never steal a building
    // or warp placement in progress.
    if matches!(
        *mode,
        InputMode::PlacingBuild(_) | InputMode::PlacingWarp(_)
    ) {
        return;
    }
    let Some(local) = session.local_player() else {
        return;
    };
    if selection.get(local).is_empty() {
        return;
    }
    let armed = if keys.just_pressed(KeyCode::KeyA) {
        TargetedOrder::Attack
    } else if keys.just_pressed(KeyCode::KeyR) {
        TargetedOrder::Patrol
    } else if keys.just_pressed(KeyCode::KeyG) {
        TargetedOrder::Guard
    } else if keys.just_pressed(KeyCode::KeyT) {
        TargetedOrder::Follow
    } else if keys.just_pressed(KeyCode::KeyB) {
        TargetedOrder::Board
    } else if keys.just_pressed(KeyCode::KeyQ) {
        TargetedOrder::AttackGround
    } else {
        return;
    };
    *mode = InputMode::Targeting(armed);
}

/// What an armed skill would be cast on, were the player to click now.
#[derive(Resource, Default)]
pub struct AimHover(pub Option<SkillTarget>);

/// What the cast the player is aiming would make of what the cursor is over.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimVerdict {
    /// No skill armed, nothing under the cursor, or no caster to judge for.
    #[default]
    Idle,
    /// The cast would take what the cursor is over.
    Takes,
    /// The cast would refuse what the cursor is over, for this reason.
    Refuses(AimRefusal),
    /// The cast takes what the cursor is over, but the leading caster cannot
    /// get within its reach of it.
    OutOfReach,
}

/// While a skill is armed, notes what the cursor would cast it on.
pub fn hover_aim(
    mode: Res<InputMode>,
    registry: Res<ContentRegistry>,
    mut hover: ResMut<AimHover>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
) {
    let target = match &*mode {
        &InputMode::Targeting(TargetedOrder::Skill(skill)) => {
            let cursor = windows.single().ok().zip(cameras.single().ok()).and_then(
                |(window, (camera, camera_transform))| {
                    cursor_world(window, camera, camera_transform)
                },
            );
            cursor.and_then(|cursor| skill_target_at(cursor, skill, &registry, &entities))
        }
        InputMode::Normal
        | InputMode::PlacingBuild(_)
        | InputMode::PlacingWarp(_)
        | InputMode::Targeting(
            TargetedOrder::Attack
            | TargetedOrder::Patrol
            | TargetedOrder::Guard
            | TargetedOrder::Follow
            | TargetedOrder::Board
            | TargetedOrder::Load
            | TargetedOrder::Unload
            | TargetedOrder::AttackGround,
        ) => None,
    };
    if hover.0 != target {
        hover.0 = target;
    }
}

/// Judges the hovered aim of an armed skill by the simulation's own rule, for
/// the leading caster.
pub fn judge_aim(world: &mut World) {
    let verdict = match world.resource::<InputMode>() {
        &InputMode::Targeting(TargetedOrder::Skill(skill)) => {
            let hovered = world.resource::<AimHover>().0;
            let player = world.resource::<GameSession>().local_player();
            let caster = world
                .resource::<Leading>()
                .0
                .and_then(|id| world.resource::<EntityIndex>().interactable(world, id));
            match (hovered, player, caster) {
                (Some(target), Some(player), Some(caster)) => {
                    match cast::aim(world, player, caster, skill, Some(target)) {
                        Ok(aim) if cast::reaches_aim(world, caster, skill, aim) => {
                            AimVerdict::Takes
                        }
                        Ok(_) => AimVerdict::OutOfReach,
                        Err(refusal) => AimVerdict::Refuses(refusal),
                    }
                }
                (None, _, _) | (_, None, _) | (_, _, None) => AimVerdict::Idle,
            }
        }
        InputMode::Normal
        | InputMode::PlacingBuild(_)
        | InputMode::PlacingWarp(_)
        | InputMode::Targeting(
            TargetedOrder::Attack
            | TargetedOrder::Patrol
            | TargetedOrder::Guard
            | TargetedOrder::Follow
            | TargetedOrder::Board
            | TargetedOrder::Load
            | TargetedOrder::Unload
            | TargetedOrder::AttackGround,
        ) => AimVerdict::Idle,
    };
    *world.resource_mut::<AimVerdict>() = verdict;
}

/// While a combat order is armed, left-click issues it (Esc/RMB cancel).
pub fn targeting_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    registry: Res<ContentRegistry>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    leading: Res<Leading>,
    aim_verdict: Res<AimVerdict>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
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
        TargetedOrder::Attack => {
            // An entity under the cursor is attacked as named — the executor
            // honours the order against allies too; an empty cell is attacked
            // toward, engaging whatever is met on the way.
            match entity_at(cursor, Clicked::Standing, &registry, &entities) {
                Some(target) => pending.push(PlayerCommand::Attack {
                    target: AttackTarget::Entity(target),
                    flush,
                }),
                None => pending.push(PlayerCommand::AttackMove {
                    target: world_to_pos(cursor),
                    flush,
                }),
            }
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
            let Some(target) = entity_at(cursor, Clicked::Standing, &registry, &entities) else {
                return;
            };
            pending.push(PlayerCommand::Guard { target, flush });
        }
        TargetedOrder::Follow => {
            let Some(target) = entity_at(cursor, Clicked::Standing, &registry, &entities) else {
                return;
            };
            pending.push(PlayerCommand::Follow { target, flush });
        }
        TargetedOrder::Board => {
            let Some(target) = entity_at(cursor, Clicked::Standing, &registry, &entities) else {
                return;
            };
            pending.push(PlayerCommand::Board { target, flush });
        }
        TargetedOrder::AttackGround => {
            pending.push(PlayerCommand::Attack {
                target: AttackTarget::Position(world_to_pos(cursor)),
                flush,
            });
        }
        TargetedOrder::Load => {
            // The fetch needs an entity under the cursor; a miss keeps the mode
            // armed so the player can click again.
            let Some(target) = entity_at(cursor, Clicked::Standing, &registry, &entities) else {
                return;
            };
            if let Some(holder) = leading.0 {
                pending.push(PlayerCommand::Load {
                    holder,
                    target,
                    flush,
                });
            }
        }
        TargetedOrder::Unload => {
            if let Some(holder) = leading.0 {
                pending.push(PlayerCommand::Unload {
                    holder,
                    at: Some(world_to_pos(cursor)),
                    flush,
                });
            }
        }
        TargetedOrder::Skill(skill) => {
            let Some(local) = session.local_player() else {
                return;
            };
            // A miss, or a target the cast would refuse, keeps the mode armed
            // so the player can click again; the hint line says why.
            let Some(target) = skill_target_at(cursor, skill, &registry, &entities) else {
                return;
            };
            match *aim_verdict {
                AimVerdict::Refuses(_) | AimVerdict::OutOfReach => return,
                AimVerdict::Takes | AimVerdict::Idle => {}
            }
            for &caster in selection.get(local) {
                pending.push(PlayerCommand::UseSkill {
                    skill,
                    caster: SkillCasterRef::Entity(caster),
                    target: Some(target),
                });
            }
        }
    }
    *mode = InputMode::Normal;
}

/// `X` cycles the selection's stance, starting from the leading entity's.
pub fn stance_input(
    keys: Res<ButtonInput<KeyCode>>,
    leading: Res<Leading>,
    mut pending: ResMut<PendingInput>,
    stances: Query<(&EntityInfoComponent, &StanceComponent)>,
) {
    if !keys.just_pressed(KeyCode::KeyX) {
        return;
    }
    let Some(id) = leading.0 else {
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
    let Some(local) = session.local_player() else {
        return;
    };
    let ids = groups.get(local, group);
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
///
/// The ghost is judged by what the perspective knows — see [`GhostView`] —
/// against the type's field rules. What stands there unseen refuses the raise
/// when it starts, never the ghost.
pub fn placement_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<InputMode>,
    mut pending: ResMut<PendingInput>,
    mut gizmos: Gizmos,
    (leading, yielders): (Res<Leading>, Res<Yielders>),
    map: Res<Map>,
    registry: Res<ContentRegistry>,
    session: Res<GameSession>,
    (fields, fog, ghosts): (Res<FieldGrid>, Res<VisibilityGrid>, Res<render::Ghosts>),
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    sources: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &Visibility,
        Option<&render::Sighted>,
        Option<&OwnerComponent>,
        Option<&FieldSourcesComponent>,
        Option<&AttachedComponent>,
    )>,
) {
    let InputMode::PlacingBuild(type_name) = &*mode else {
        return;
    };
    let type_name = type_name.clone();
    let Some(local) = session.local_player() else {
        return;
    };

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
    let Some(def) = registry.entity(&type_name) else {
        *mode = InputMode::Normal;
        return;
    };
    let location_def = def.location.expect("validated content defines a location");
    let size = location_def.size();

    // What the perspective knows of the ground, and the fields the type
    // reads; a type raised over a source snaps to one under the cursor.
    let cursor = CellPos::new(cx, cy);
    let mut made_out = made_out_footprints(
        &registry,
        sources
            .iter()
            .filter(|(info, ..)| {
                !ghost_ignores(&registry, leading.0, &yielders, info.id(), info.type_id())
            })
            .map(|(info, location, visibility, sighted, _, _, seated)| {
                (info, location, visibility, sighted, seated)
            }),
    );
    made_out.extend(ghosts.remembered_footprints());
    let ghost = GhostView {
        map: &map,
        registry: &registry,
        fields: &fields,
        fog: &fog,
        session: &session,
        player: local,
        made_out: &made_out,
    };
    let (anchor, passable) = ghost.placed(
        def,
        cursor,
        sources
            .iter()
            .map(|(info, location, visibility, sighted, _, _, _)| {
                (info, location, visibility, sighted)
            }),
    );
    let (cx, cy) = (anchor.x, anchor.y);

    // The area each field the type projects would cover, and the areas the
    // standing sources of those fields already cover, so a pylon is placed
    // where it links up and a tumor where it reaches.
    for source in &def.field_sources {
        let color = render::field_reach_color(&registry, source.field());
        draw_reach(
            &mut gizmos,
            CellPos::new(cx, cy),
            size,
            source.radius(),
            color,
        );
        // Other sources of the same field, at what they put on the grid: the
        // side's own, allies included — another side's reach is its own to
        // know, whatever the fog shows of the source itself.
        for (info, location, _, _, owner, states, _) in &sources {
            let own_side = owner.is_some_and(|owner| session.are_allied(local, owner.player()));
            let Some(states) = states else {
                continue;
            };
            if !own_side {
                continue;
            }
            let cell = body::anchor(location.position);
            let standing = registry.def(info.type_id());
            for (other, state) in standing.field_sources.iter().zip(&states.0) {
                if other.field() != source.field() {
                    continue;
                }
                let Emitted::Reach(reach) = state.last_emitted else {
                    continue;
                };
                draw_reach(
                    &mut gizmos,
                    cell,
                    standing
                        .location
                        .expect("validated content defines a location")
                        .size(),
                    reach,
                    color.with_alpha(0.35),
                );
            }
        }
    }
    draw_footprint_ghost(&mut gizmos, CellRect::new(anchor, size), passable);

    if mouse.just_pressed(MouseButton::Left)
        && let Some(builder) = leading.0
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

/// While warping a unit in, draw its footprint ghost and record a left-click
/// as the warp-in to give a warper (Esc/RMB cancel).
///
/// The ghost is judged by what the perspective knows — see [`GhostView`] —
/// against the leading warper's field rules and the unit's own, and the
/// landing word. What stands there unseen refuses the warp, never the ghost.
#[allow(clippy::too_many_arguments)]
pub fn warp_placement_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<InputMode>,
    mut click: ResMut<WarpClick>,
    mut gizmos: Gizmos,
    leading: Res<Leading>,
    (map, registry, session, fields, visibility, index): (
        Res<Map>,
        Res<ContentRegistry>,
        Res<GameSession>,
        Res<FieldGrid>,
        Res<VisibilityGrid>,
        Res<EntityIndex>,
    ),
    infos: Query<&EntityInfoComponent>,
    seen: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &Visibility,
        Option<&render::Sighted>,
        Option<&AttachedComponent>,
    )>,
    ghosts: Res<render::Ghosts>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let InputMode::PlacingWarp(type_name) = &*mode else {
        return;
    };
    let type_name = type_name.clone();
    let Some(local) = session.local_player() else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right) {
        *mode = InputMode::Normal;
        return;
    }
    // Neither ghost nor warp under the cursor while it is over a HUD button —
    // in particular, the card button's own click that just entered this mode.
    if pointer_over_ui(&interactions) {
        return;
    }
    let warper_def = leading
        .0
        .and_then(|id| index.alive(id))
        .and_then(|entity| infos.get(entity).ok())
        .and_then(|info| registry.def(info.type_id()).warper.as_ref());
    let (Some(warper), Some(warper_def), Some(def)) =
        (leading.0, warper_def, registry.entity(&type_name))
    else {
        *mode = InputMode::Normal;
        return;
    };
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = cursor_world(window, camera, camera_transform) else {
        return;
    };
    let Some((cx, cy)) = world_to_cell(cursor) else {
        return;
    };
    let size = def
        .location
        .expect("validated content defines a location")
        .size();
    let mut made_out = made_out_footprints(&registry, seen.iter());
    made_out.extend(ghosts.remembered_footprints());
    let ghost = GhostView {
        map: &map,
        registry: &registry,
        fields: &fields,
        fog: &visibility,
        session: &session,
        player: local,
        made_out: &made_out,
    };
    // A type raised over a source snaps to one under the cursor, as a build
    // ghost does.
    let (anchor, placed) = ghost.placed(
        def,
        CellPos::new(cx, cy),
        seen.iter()
            .map(|(info, location, visibility, sighted, _)| (info, location, visibility, sighted)),
    );
    let footprint = CellRect::new(anchor, size);
    let fits = placed
        && ghost.allows(warper_def.within(), footprint)
        && landing::sees_in(
            &visibility,
            &session,
            local,
            warper_def.landing(),
            footprint,
        );

    draw_footprint_ghost(&mut gizmos, footprint, fits);

    if mouse.just_pressed(MouseButton::Left) {
        click.0 = Some(Warp {
            leading: warper,
            type_name,
            at: FixedUVec2::from(anchor),
        });
        *mode = InputMode::Normal;
    }
}

/// While a summon aimed at a position is armed, mark the cell under the
/// cursor: an exact summon draws the summoned type's footprint ghost there,
/// fitting where the cast takes that aim — its reach — and the ground there
/// takes the body as the leading caster's side knows it, inside the summon's
/// field rules and its landing word; a nearby summon a pointer on the cell,
/// as its search sets the body down beside it.
///
/// The click is [`targeting_input`]'s.
#[allow(clippy::too_many_arguments)]
pub fn summon_ghost(
    mode: Res<InputMode>,
    verdict: Res<AimVerdict>,
    mut gizmos: Gizmos,
    (map, registry, session, fields, visibility): (
        Res<Map>,
        Res<ContentRegistry>,
        Res<GameSession>,
        Res<FieldGrid>,
        Res<VisibilityGrid>,
    ),
    seen: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &Visibility,
        Option<&render::Sighted>,
        Option<&AttachedComponent>,
    )>,
    ghosts: Res<render::Ghosts>,
    interactions: Query<&Interaction>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let &InputMode::Targeting(TargetedOrder::Skill(skill)) = &*mode else {
        return;
    };
    let Some(SkillCaster::Entity {
        effect:
            EntityCastEffect::Summon {
                at: SummonAt::Cell { .. },
                entity_type,
                placement,
                landing,
                within,
                ..
            },
        ..
    }) = registry.skill_def(skill).map(|def| &def.caster)
    else {
        return;
    };
    let Some(local) = session.local_player() else {
        return;
    };
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
    match placement {
        SummonPlacement::Exact => {}
        SummonPlacement::Nearby => {
            let center = Vec2::new((cx as f32 + 0.5) * CELL_PX, -(cy as f32 + 0.5) * CELL_PX);
            gizmos.circle_2d(
                Isometry2d::from_translation(center),
                CELL_PX * 0.35,
                Color::srgb(1.0, 0.85, 0.35),
            );
            return;
        }
    }
    let def = registry.def(*entity_type);
    let size = def
        .location
        .expect("validated content defines a location")
        .size();
    let mut made_out = made_out_footprints(&registry, seen.iter());
    made_out.extend(ghosts.remembered_footprints());
    let ghost = GhostView {
        map: &map,
        registry: &registry,
        fields: &fields,
        fog: &visibility,
        session: &session,
        player: local,
        made_out: &made_out,
    };
    let (anchor, placed) = ghost.placed(
        def,
        CellPos::new(cx, cy),
        seen.iter()
            .map(|(info, location, visibility, sighted, _)| (info, location, visibility, sighted)),
    );
    let footprint = CellRect::new(anchor, size);
    let takes_aim = match *verdict {
        AimVerdict::Takes => true,
        AimVerdict::Idle | AimVerdict::Refuses(_) | AimVerdict::OutOfReach => false,
    };
    let fits = takes_aim
        && placed
        && ghost.allows(within, footprint)
        && landing::sees_in(&visibility, &session, local, *landing, footprint);
    draw_footprint_ghost(&mut gizmos, footprint, fits);
}

/// A warp-in clicked this frame, not yet given to a warper.
#[derive(Resource, Default)]
pub struct WarpClick(pub Option<Warp>);

/// A warp-in the player clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warp {
    /// The leading warper the ghost was judged by.
    pub leading: SimulationId,
    /// The unit's type, by name.
    pub type_name: String,
    /// Where it was clicked.
    pub at: FixedUVec2,
}

/// Warp-ins given to warpers whose commands may not have landed yet.
#[derive(Resource, Default)]
pub struct WarpsSent(pub Vec<SentWarp>);

/// A warp-in given to a warper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentWarp {
    /// The warper given the warp-in.
    pub warper: SimulationId,
    /// The tick it was given it.
    pub tick: u32,
    /// The cooldown end the warper held then.
    pub cooldown_end: u32,
}

impl SentWarp {
    /// Whether its command may still be on its way at `tick`: the tick it
    /// was given for has not surely run yet.
    pub fn in_flight(&self, tick: u32) -> bool {
        tick <= self.tick + SYNC_LATENCY + 1
    }
}

/// Gives the warp-in clicked this frame to the first of its candidates that
/// may warp the type in now — see [`warp_candidates`] — else to the leading
/// warper, whose refusal the simulation answers.
pub fn issue_warp(world: &mut World) {
    let Some(warp) = world.resource_mut::<WarpClick>().0.take() else {
        return;
    };
    let Some(local) = world.resource::<GameSession>().local_player() else {
        return;
    };
    let ready = warp_candidates(world, local, warp.leading)
        .into_iter()
        .find(|&entity| warp::judge_warper(world, local, entity, &warp.type_name).is_ok());
    let warper = ready
        .map(|entity| entity_def::simulation_id(world, entity))
        .unwrap_or(warp.leading);
    // Kept while the command may still be on its way, until its tick has
    // surely run; one whose warper's cooldown has moved is passed over by
    // `warp_candidates`.
    let tick = world.resource::<GameSession>().tick();
    let mut sent = std::mem::take(&mut world.resource_mut::<WarpsSent>().0);
    sent.retain(|sent_warp| sent_warp.in_flight(tick));
    if let Some(entity) = ready {
        sent.push(SentWarp {
            warper,
            tick,
            cooldown_end: cooldown_end(world, entity),
        });
    }
    world.resource_mut::<WarpsSent>().0 = sent;
    world
        .resource_mut::<PendingInput>()
        .push(PlayerCommand::WarpIn {
            warper,
            type_name: warp.type_name,
            at: warp.at,
        });
}

/// The local player's warpers a warp-in led by `leading` may go to, in the
/// order they are offered it: the leading warper, then the other selected own
/// warpers of its type in selection order — the ghost's rules are theirs —
/// leaving out any given a warp-in whose command has not landed yet.
pub fn warp_candidates(world: &World, local: PlayerId, leading: SimulationId) -> Vec<Entity> {
    let index = world.resource::<EntityIndex>();
    let Some(leader) = index.interactable(world, leading) else {
        return Vec::new();
    };
    let warper_type = entity_def::type_id(world, leader);
    let tick = world.resource::<GameSession>().tick();
    let still_sent = |id: SimulationId, entity: Entity| {
        world.get_resource::<WarpsSent>().is_some_and(|sent| {
            sent.0.iter().any(|sent_warp| {
                sent_warp.warper == id
                    && sent_warp.in_flight(tick)
                    && cooldown_end(world, entity) == sent_warp.cooldown_end
            })
        })
    };
    std::iter::once(leading)
        .chain(
            world
                .resource::<Selection>()
                .get(local)
                .iter()
                .copied()
                .filter(|&id| id != leading),
        )
        .filter_map(|id| index.interactable(world, id).map(|entity| (id, entity)))
        .filter(|&(id, entity)| {
            entity_def::owner(world, entity) == Some(local)
                && entity_def::type_id(world, entity) == warper_type
                && entity_def::warper_def(world, entity).is_some()
                && !still_sent(id, entity)
        })
        .map(|(_, entity)| entity)
        .collect()
}

/// The tick `warper`'s cooldown runs out at.
fn cooldown_end(world: &World, warper: Entity) -> u32 {
    world
        .get::<WarpComponent>(warper)
        .expect("a warper keeps its cooldown")
        .ready_at
}

/// What a placement ghost is judged against: what `player`'s side knows of
/// the ground and the fields.
pub struct GhostView<'a> {
    /// The map whose terrain the ghost stands on.
    pub map: &'a Map,
    /// The content the rules are read from.
    pub registry: &'a ContentRegistry,
    /// The fields, read as far as the overlay shows them.
    pub fields: &'a FieldGrid,
    /// The fog the player sees the fields through.
    pub fog: &'a VisibilityGrid,
    /// The session the player sits in.
    pub session: &'a GameSession,
    /// The player placing.
    pub player: PlayerId,
    /// The footprints the player makes out or remembers.
    pub made_out: &'a [render::MadeOutFootprint],
}

impl GhostView<'_> {
    /// Whether the ghost of `def` anchored at `anchor` shows as fitting: its
    /// footprint on the map, over ground the simulation's [`ground::takes`]
    /// says it takes wherever the side has explored it; clear, on a layer it
    /// occupies, of every footprint made out that claims its cells — of
    /// standing ones alone for a body that claims none — and, for a footprint
    /// on the static plane, of every ground holder that keeps it off; and its
    /// type's field rules met.
    pub fn fits(&self, def: &EntityTypeDef, anchor: CellPos) -> bool {
        let location_def = def.location.expect("validated content defines a location");
        let shape = entity_def::mover_shape(self.registry, &location_def);
        let placed = CellRect::new(anchor, shape.size);
        let class = OccupancyClass::of(def);
        placed.cells().all(|cell| self.map.contains(cell))
            && self.takes_explored(anchor, shape, class)
            && !self.made_out.iter().any(|footprint| {
                footprint.layers & shape.layers != LayerMask::EMPTY
                    && footprint.rect.intersects(placed)
                    && match (footprint.solidity, footprint.class) {
                        // A body claiming its cells needs them clear of every
                        // claim; one claiming none, of standing claims alone.
                        (Solidity::Solid, OccupancyClass::Static) => true,
                        (Solidity::Solid, OccupancyClass::Claim) => match location_def.solidity() {
                            Solidity::Solid => true,
                            Solidity::Underfoot | Solidity::Passable => false,
                        },
                        // A mover claiming nothing keeps solid standing
                        // footprints off.
                        (Solidity::Underfoot | Solidity::Passable, OccupancyClass::Claim) => {
                            match (class, location_def.solidity()) {
                                (OccupancyClass::Static, Solidity::Solid) => true,
                                (
                                    OccupancyClass::Static,
                                    Solidity::Underfoot | Solidity::Passable,
                                )
                                | (
                                    OccupancyClass::Claim,
                                    Solidity::Solid | Solidity::Underfoot | Solidity::Passable,
                                ) => false,
                            }
                        }
                        // A standing ground holder keeps off what its
                        // solidity keeps off.
                        (Solidity::Underfoot | Solidity::Passable, OccupancyClass::Static) => {
                            match class {
                                OccupancyClass::Static => {
                                    footprint.solidity.keeps_off(location_def.solidity())
                                }
                                OccupancyClass::Claim => false,
                            }
                        }
                    }
            })
            && self.allows(&def.field_placement, placed)
    }

    /// Where the ghost of `def` stands with the cursor on `cursor`, and whether
    /// it shows as fitting there. A type raised over a resource source snaps
    /// to the footprint of a source of that type among `sources` the
    /// perspective makes out under the cursor — that footprint is its ground —
    /// and fits as its field rules say; with none under the cursor it stays
    /// there and does not fit. Any other type stands on `cursor` and fits as
    /// [`fits`](Self::fits) says.
    pub fn placed<'a>(
        &self,
        def: &EntityTypeDef,
        cursor: CellPos,
        sources: impl IntoIterator<
            Item = (
                &'a EntityInfoComponent,
                &'a LocationComponent,
                &'a Visibility,
                Option<&'a render::Sighted>,
            ),
        >,
    ) -> (CellPos, bool) {
        let Some(over) = def.overbuilds.as_deref() else {
            return (cursor, self.fits(def, cursor));
        };
        let size = def
            .location
            .expect("validated content defines a location")
            .size();
        let under = sources
            .into_iter()
            .find_map(|(info, location, visibility, sighted)| {
                let standing = self.registry.def(info.type_id());
                snaps_to(over, cursor, info, location, standing, visibility, sighted)
            });
        match under {
            Some(anchor) => (
                anchor,
                self.allows(&def.field_placement, CellRect::new(anchor, size)),
            ),
            None => (cursor, false),
        }
    }

    /// Whether `rules` admit a footprint at `rect` on the field coverage the
    /// overlay shows the player — see [`render::shown_coverage`].
    pub fn allows(&self, rules: &[FieldPlacement], rect: CellRect) -> bool {
        let layers = render::shown_fields(self.registry);
        let sight = render::CoverageSight {
            session: self.session,
            viewed: Some(self.player),
            fields: self.fields,
            fog: self.fog,
            reveal: FogReveal::Fogged,
        };
        fields::rules_allow_by(
            self.session,
            Some(self.player),
            rules,
            rect,
            |field, cell| {
                self.fields
                    .contains(cell)
                    .then(|| render::shown_coverage(&sight, &layers, field, cell))
            },
        )
    }

    /// Whether the ground the side has explored takes a footprint of `shape`
    /// at `anchor` on the plane `occupancy` selects, as [`ground::takes`]
    /// judges it, a cell never explored read as taking it: the whole
    /// footprint when every cell is explored, else each explored cell alone,
    /// on one level for a static footprint.
    fn takes_explored(
        &self,
        anchor: CellPos,
        shape: MoverShape,
        occupancy: OccupancyClass,
    ) -> bool {
        let footprint = CellRect::new(anchor, shape.size);
        let explored =
            |cell: CellPos| match self
                .fog
                .visibility_to(self.session, self.player, cell.x, cell.y)
            {
                CellVisibility::Unexplored => false,
                CellVisibility::Explored | CellVisibility::Visible => true,
            };
        if footprint.cells().all(explored) {
            return ground::takes(self.map, anchor, shape, occupancy);
        }
        let single = MoverShape {
            size: CellSize::ONE,
            ..shape
        };
        let grid = self.map.nav_grid();
        let level = |cell: CellPos| grid.relief(shape.layer, cell).map(|relief| relief.level);
        let known: Vec<CellPos> = footprint.cells().filter(|&cell| explored(cell)).collect();
        let one_level = match occupancy {
            OccupancyClass::Static => known.iter().all(|&cell| level(cell) == level(known[0])),
            OccupancyClass::Claim => true,
        };
        one_level
            && known
                .iter()
                .all(|&cell| ground::takes(self.map, cell, single, occupancy))
    }
}

/// The footprint of every entity the perspective makes out standing on the
/// map — a mover claiming no cells over every cell it stands across — but
/// those seated in a berth.
pub fn made_out_footprints<'a>(
    registry: &ContentRegistry,
    entities: impl Iterator<
        Item = (
            &'a EntityInfoComponent,
            &'a LocationComponent,
            &'a Visibility,
            Option<&'a render::Sighted>,
            Option<&'a AttachedComponent>,
        ),
    >,
) -> Vec<render::MadeOutFootprint> {
    entities
        .filter(|&(_, _, visibility, sighted, seated)| {
            render::made_out(visibility, sighted) && seated.is_none()
        })
        .filter_map(|(info, location, _, _, _)| {
            let def = registry.def(info.type_id());
            let location_def = def.location?;
            let class = OccupancyClass::of(def);
            let footprint = match (class, location_def.solidity()) {
                (OccupancyClass::Claim, Solidity::Underfoot | Solidity::Passable) => {
                    body::standing_rect(location.position, location_def.size())
                }
                (OccupancyClass::Claim, Solidity::Solid)
                | (
                    OccupancyClass::Static,
                    Solidity::Solid | Solidity::Underfoot | Solidity::Passable,
                ) => CellRect::new(body::anchor(location.position), location_def.size()),
            };
            Some(render::MadeOutFootprint {
                layers: location_def.occupies(),
                rect: footprint,
                solidity: location_def.solidity(),
                class,
            })
        })
        .collect()
}

/// The footprint origin a raise over `over` snaps to when `cursor` is on a
/// source of that type the perspective sees: `info`, `location`, `standing`
/// and the stamp are the source's own.
pub fn snaps_to(
    over: &str,
    cursor: CellPos,
    info: &EntityInfoComponent,
    location: &LocationComponent,
    standing: &EntityTypeDef,
    visibility: &Visibility,
    sighted: Option<&render::Sighted>,
) -> Option<CellPos> {
    if info.type_name() != over {
        return None;
    }
    let size = standing
        .location
        .expect("validated content defines a location")
        .size();
    let footprint = CellRect::new(body::anchor(location.position), size);
    (footprint.contains(cursor) && render::made_out(visibility, sighted))
        .then_some(footprint.origin)
}

/// Whether an entity of `def` gets out of the way of a site it stands on and
/// is to raise: a builder that walks, or whose attendance takes it off the
/// grid as the site goes up.
pub fn gets_out_of_way(def: &EntityTypeDef) -> bool {
    match def.builder.as_ref().map(|builder| builder.attendance()) {
        Some(
            BuilderAttendance::Crew(WorkPresence::Hidden { .. } | WorkPresence::Attached(_))
            | BuilderAttendance::Consumed,
        ) => true,
        Some(
            BuilderAttendance::Crew(WorkPresence::Present { .. }) | BuilderAttendance::Unattended,
        ) => def.can_move(),
        None => false,
    }
}

/// Outlines the reach of a `size` footprint anchored at `anchor`: a circle
/// of `radius` cells around the footprint, whatever the map's metric.
fn draw_reach(gizmos: &mut Gizmos, anchor: CellPos, size: CellSize, radius: u32, color: Color) {
    let center = Vec2::new(
        (anchor.x as f32 + size.width as f32 / 2.0) * CELL_PX,
        -(anchor.y as f32 + size.height as f32 / 2.0) * CELL_PX,
    );
    let half = (size.width.max(size.height) as f32 / 2.0) * CELL_PX;
    let reach = radius as f32 * CELL_PX;
    gizmos.circle_2d(Isometry2d::from_translation(center), half + reach, color);
}

/// What a click at `cursor` would cast `skill` on: a cell, a body on the
/// ground, or something standing, as the skill takes — or `None` on a miss.
fn skill_target_at(
    cursor: Vec2,
    skill: SkillId,
    registry: &ContentRegistry,
    entities: &Query<(
        &EntityInfoComponent,
        &LocationComponent,
        Option<&Visibility>,
        Option<&render::Sighted>,
        Has<RemainsComponent>,
    )>,
) -> Option<SkillTarget> {
    let aim = match registry.skill_def(skill).map(|def| &def.caster) {
        Some(SkillCaster::Entity { effect, .. }) => effect.target(),
        Some(SkillCaster::Player { .. }) | None => return None,
    };
    match aim {
        EntityCastTarget::Position { .. } => Some(SkillTarget::Position(world_to_pos(cursor))),
        EntityCastTarget::Fallen { .. } => {
            entity_at(cursor, Clicked::Remains, registry, entities).map(SkillTarget::Entity)
        }
        EntityCastTarget::Caster | EntityCastTarget::Standing { .. } => {
            entity_at(cursor, Clicked::Standing, registry, entities).map(SkillTarget::Entity)
        }
    }
}

/// Draws a placement ghost over `footprint`: green when it `fits`, red when
/// not.
fn draw_footprint_ghost(gizmos: &mut Gizmos, footprint: CellRect, fits: bool) {
    let CellRect { origin, size } = footprint;
    let center = Vec2::new(
        (origin.x as f32 + size.width as f32 / 2.0) * CELL_PX,
        -(origin.y as f32 + size.height as f32 / 2.0) * CELL_PX,
    );
    let extent = Vec2::new(size.width as f32, size.height as f32) * CELL_PX;
    let color = if fits {
        Color::srgb(0.3, 1.0, 0.4)
    } else {
        Color::srgb(1.0, 0.3, 0.3)
    };
    gizmos.rect_2d(Isometry2d::from_translation(center), extent, color);
}
