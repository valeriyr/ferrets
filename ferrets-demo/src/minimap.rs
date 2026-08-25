//! The minimap: one texture holding a pixel per map cell, recomposed each
//! simulation tick from the terrain, the local team's knowledge of it, live
//! entity blips, and the camera's viewport outline — and clickable both to look
//! somewhere and to order the selection there.
//!
//! The pixel arithmetic and the widget's cell hit-test are pure functions kept
//! clear of Bevy and simulation types, so they can be exercised on their own;
//! the systems below hold everything that knows about this game's look.

use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_4};

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{RelativeCursorPosition, Val2, widget::NodeImageMode},
    window::PrimaryWindow,
};
use ferrets_bevy_plugin::PendingInput;
use ferrets_content::registry::ContentRegistry;
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, health::HealthComponent, hidden::HiddenComponent,
        location::LocationComponent, owner::OwnerComponent, rally::RallyPointComponent,
    },
    map::Map,
    selection::Selection,
    session::GameSession,
    visibility::{CellVisibility, VisibilityGrid},
};

use crate::{
    camera, input, map,
    render::{self, CELL_PX, FogReveal, Ghosts},
    scenario::CurrentScenario,
    states::InGameUi,
    view::WorldView,
};

//
// ─── Look ─────────────────────────────────────────────────────────────────────
//

/// On-screen pixels spanned by the map's longer axis.
const FRAME_PX: f32 = 192.0;

/// How much of a cell's terrain color survives where the local team has been
/// but cannot currently see. It is the complement of the alpha the world's fog
/// overlay darkens with, so the two views agree on what "explored" looks like.
const EXPLORED_DIM: f32 = 0.45;

/// A building the local team remembers but cannot currently see, matching the
/// grey the world view outlines its ghost in.
const GHOST: [u8; 4] = [140, 140, 158, 255];

/// What a cell reads as before the local team has ever seen it — the void the
/// world draws outside the playable field.
const VOID: [u8; 4] = [23, 23, 28, 255];

/// How far the diamond look flattens the picture after turning it, matching
/// the height the camera doubles.
const DIAMOND_SQUASH: f32 = 0.5;

/// How far the diamond look turns the picture — the quarter of a right angle
/// the camera turns the world, seen from the other side.
const DIAMOND_TURN: f32 = FRAC_PI_4;

/// The camera's viewport outline.
const VIEWPORT: [u8; 4] = [235, 235, 235, 255];

/// A blip whose entity was hurt a moment ago.
const HURT: [u8; 4] = [255, 60, 50, 255];

/// A blip belonging to the current selection.
const SELECTED: [u8; 4] = [255, 255, 255, 255];

/// Ticks a blip keeps answering for a hit — seven seconds at the nominal
/// cadence, counted in ticks because it marks something the simulation did.
const HURT_TICKS: u32 = 140;

/// Ticks at the start of that during which the blip stays solid rather than
/// blinking, so a single hit registers even if it lands mid-blink.
const HURT_SOLID_TICKS: u32 = 20;

/// Ticks per half-cycle of the blink — a full cycle a second.
const HURT_PHASE_TICKS: u32 = 10;

//
// ─── Mechanism: pixel arithmetic and the widget's hit-test ────────────────────
//

/// An RGBA pixel buffer holding one pixel per map cell, addressed in cells from
/// the map's top-left corner.
pub struct Canvas {
    /// Cells across.
    width: u32,
    /// Cells down.
    height: u32,
    /// Four bytes per cell, row-major.
    bytes: Vec<u8>,
}

impl Canvas {
    /// A canvas of `width` × `height` cells, every cell painted `fill`.
    pub fn new(width: u32, height: u32, fill: [u8; 4]) -> Self {
        Self {
            width,
            height,
            bytes: fill.repeat((width as usize) * (height as usize)),
        }
    }

    /// Cells across.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Cells down.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The painted bytes, four per cell.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The color of one cell, or `None` past the canvas edge.
    pub fn get(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let at = self.offset(x, y)?;
        Some([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ])
    }

    /// Paints one cell. Coordinates past the canvas edge paint nothing.
    pub fn put(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if let Some(at) = self.offset(x, y) {
            self.bytes[at..at + 4].copy_from_slice(&color);
        }
    }

    /// Paints the `width` × `height` block of cells anchored at `(x, y)`,
    /// clipped at the canvas edge.
    pub fn fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
        for dy in 0..height {
            for dx in 0..width {
                self.put(x + dx, y + dy, color);
            }
        }
    }

    /// Paints the run of cells from `from` to `to`, both ends included. Points
    /// outside the canvas are skipped rather than clamped, so a line leaving
    /// the map keeps the slope it would have had.
    pub fn line(&mut self, from: IVec2, to: IVec2, color: [u8; 4]) {
        let delta = (to - from).abs();
        let step = IVec2::new(
            if from.x < to.x { 1 } else { -1 },
            if from.y < to.y { 1 } else { -1 },
        );
        let mut at = from;
        let mut error = delta.x - delta.y;
        loop {
            if at.x >= 0 && at.y >= 0 {
                self.put(at.x as u32, at.y as u32, color);
            }
            if at == to {
                return;
            }
            // Bresenham: whichever axis is further behind takes the next step,
            // and a diagonal run steps both at once.
            let doubled = error * 2;
            if doubled > -delta.y {
                error -= delta.y;
                at.x += step.x;
            }
            if doubled < delta.x {
                error += delta.x;
                at.y += step.y;
            }
        }
    }

    /// Paints the closed outline through `corners` in order. The corners are
    /// kept as four separate points rather than reduced to a rectangle: a
    /// turned quad's bounding box covers about twice the ground it does.
    pub fn outline(&mut self, corners: [IVec2; 4], color: [u8; 4]) {
        let mut previous = corners[3];
        for corner in corners {
            self.line(previous, corner, color);
            previous = corner;
        }
    }

    /// Repaints every cell from `other`, which must match this canvas's size.
    pub fn restore(&mut self, other: &Canvas) {
        debug_assert_eq!(self.bytes.len(), other.bytes.len());
        self.bytes.copy_from_slice(&other.bytes);
    }

    /// Where a cell's bytes start, or `None` past the canvas edge.
    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height)
            .then(|| ((y as usize) * (self.width as usize) + x as usize) * 4)
    }
}

/// The widget's on-screen size for a map of `cells`: the longer axis spans
/// `frame` pixels and the shorter one keeps the map's proportions, so a map
/// that is not square letterboxes rather than stretching.
pub fn widget_size(frame: f32, cells: (u32, u32)) -> Vec2 {
    let longer = cells.0.max(cells.1);
    if longer == 0 {
        return Vec2::ZERO;
    }
    Vec2::new(
        frame * cells.0 as f32 / longer as f32,
        frame * cells.1 as f32 / longer as f32,
    )
}

/// The cell under `normalized`, a position within the widget running from
/// `(0, 0)` at its top-left corner to `(1, 1)` at its bottom-right. `None` once
/// the position leaves the widget — a click can land in the frame around the
/// map without landing on the map.
pub fn cell_at(cells: (u32, u32), normalized: Vec2) -> Option<(u32, u32)> {
    if !(0.0..1.0).contains(&normalized.x) || !(0.0..1.0).contains(&normalized.y) {
        return None;
    }
    let cell =
        |along: f32, count: u32| ((along * count as f32) as u32).min(count.saturating_sub(1));
    (cells.0 > 0 && cells.1 > 0).then(|| (cell(normalized.x, cells.0), cell(normalized.y, cells.1)))
}

/// Dims a color toward black, keeping `factor` of it; `1.0` leaves it alone.
pub fn dim(color: [u8; 4], factor: f32) -> [u8; 4] {
    let scale = |channel: u8| (channel as f32 * factor) as u8;
    [scale(color[0]), scale(color[1]), scale(color[2]), color[3]]
}

/// A color's sRGB bytes.
pub fn bytes_of(color: Color) -> [u8; 4] {
    let srgba = color.to_srgba();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        channel(srgba.red),
        channel(srgba.green),
        channel(srgba.blue),
        channel(srgba.alpha),
    ]
}

/// How far a turned widget reaches past the frame it replaces, on each side, and
/// so how far it must shift to stay on screen. An eighth turn spreads a
/// `w` × `h` widget across `(w + h) / √2`, of which the frame already accounts
/// for `w`; the flattening then halves the height. Zero on an axis the diamond
/// does not outgrow — a square map's flattened diamond is shorter than its
/// frame, while one that is not square outgrows it on both.
pub fn overhang(frame: f32, cells: (u32, u32)) -> Vec2 {
    let size = widget_size(frame, cells);
    let across = (size.x + size.y) * FRAC_1_SQRT_2;
    Vec2::new(
        (across - size.x) / 2.0,
        (across * DIAMOND_SQUASH - size.y) / 2.0,
    )
    .max(Vec2::ZERO)
}

/// The cell a world point sits in, as canvas coordinates that may fall outside
/// the map — the viewport reaches past the field's edges when zoomed out, and
/// the canvas clips what leaves it.
pub fn cell_of(world: Vec2) -> IVec2 {
    IVec2::new(
        (world.x / CELL_PX).floor() as i32,
        (-world.y / CELL_PX).floor() as i32,
    )
}

/// The world point at the center of a cell.
fn cell_center(cell: (u32, u32)) -> Vec2 {
    Vec2::new(
        (cell.0 as f32 + 0.5) * CELL_PX,
        -(cell.1 as f32 + 0.5) * CELL_PX,
    )
}

//
// ─── State ────────────────────────────────────────────────────────────────────
//

/// The picture the minimap widget draws, and what a refresh rebuilds it from.
#[derive(Resource)]
pub struct Minimap {
    /// The texture the widget draws.
    image: Handle<Image>,
    /// Terrain color per cell, baked once — the layer that never changes.
    base: Canvas,
    /// The buffer a refresh composes into before it reaches the texture.
    canvas: Canvas,
    /// The tick the texture was last composed for.
    composed: Option<u32>,
    /// The viewport outline the texture was last composed with. The camera
    /// moves between ticks — panning a paused game is the whole point of the
    /// drag — so the tick alone does not say whether the picture is current.
    viewport: Option<[IVec2; 4]>,
}

impl Minimap {
    /// The picture as last composed.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// The map's size in cells, which is also the texture's in pixels.
    fn cells(&self) -> (u32, u32) {
        (self.canvas.width(), self.canvas.height())
    }
}

/// Marks the widget drawing the minimap texture.
#[derive(Component)]
pub struct MinimapNode;

/// Marks the frame the picture sits in, which holds its placement and — under
/// the diamond look — the flattening applied after the turn.
#[derive(Component)]
pub struct MinimapFrame;

//
// ─── Setup / teardown ─────────────────────────────────────────────────────────
//

/// Builds the minimap texture and widget for the game being opened, baking the
/// terrain layer from the map's cells.
pub fn spawn_minimap(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    session: Res<GameSession>,
    scenario: Option<Res<CurrentScenario>>,
) {
    let Some(data) = map::opened(&session, scenario.as_deref()) else {
        return;
    };
    let (width, height) = (data.width(), data.height());

    let mut base = Canvas::new(width, height, VOID);
    for (index, &terrain) in data.terrain_cells().iter().enumerate() {
        let Some(name) = data.terrains().get(terrain as usize) else {
            continue;
        };
        let index = index as u32;
        base.put(
            index % width,
            index / width,
            bytes_of(render::terrain_color(name)),
        );
    }

    let mut image = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &VOID,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // One texture pixel is one cell, blown up many times over: filtering it
    // would smear neighbouring cells into each other.
    image.sampler = ImageSampler::nearest();

    let size = widget_size(FRAME_PX, (width, height));
    let handle = images.add(image);
    // Two nodes, because the diamond look needs the picture turned and *then*
    // flattened, and one `UiTransform` can only scale before it rotates. The
    // frame owns the flattening and the placement, the picture owns the turn.
    commands
        .spawn((
            InGameUi,
            MinimapFrame,
            Node {
                position_type: PositionType::Absolute,
                // Clear of the Leave button in the same corner.
                bottom: Val::Px(44.0),
                right: Val::Px(12.0),
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..default()
            },
        ))
        .with_children(|frame| {
            frame.spawn((
                MinimapNode,
                ImageNode {
                    image: handle.clone(),
                    // One pixel per cell, stretched over the widget. Stated
                    // rather than left to the default, which sizes itself from
                    // the texture and would depend on the layout resolving the
                    // widget's own size first.
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                // Interaction is what keeps a click on the minimap from also
                // reaching the map beneath it, and the relative position is
                // what says which cell was clicked. Both are read through the
                // node's own transform, so a turned picture is hit-tested as
                // the diamond it looks like rather than as its bounding box.
                Interaction::default(),
                RelativeCursorPosition::default(),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ));
        });

    commands.insert_resource(Minimap {
        image: handle,
        canvas: Canvas::new(width, height, VOID),
        base,
        composed: None,
        viewport: None,
    });
}

/// Drops the minimap's texture and state when the game ends; the widget itself
/// goes with the rest of the in-game overlay.
pub fn teardown_minimap(mut commands: Commands) {
    commands.remove_resource::<Minimap>();
}

//
// ─── Refresh ──────────────────────────────────────────────────────────────────
//

/// Points the picture the way the camera points it.
///
/// The camera turns the world a quarter of a right angle and doubles its height
/// for the diamond look, so the world *appears* turned the other way and half as
/// tall; the picture takes that same appearance, which is why it reads as a
/// diamond alongside the field rather than as a square beside it. The turn comes
/// first and the flattening second — the reverse order flattens the map before
/// turning it, which is a different shape altogether — so the two live on
/// separate nodes.
///
/// The turn is not paid for by shrinking the picture, so the diamond reaches
/// past the frame it replaces and the widget shifts inward by that overhang to
/// stay clear of the window's edge. The overhang is measured from the widget's
/// own proportions, which a map that is not square does not share with the
/// frame.
///
/// The flat looks take the identity, so this costs them nothing.
pub fn follow_view(
    view: Res<WorldView>,
    minimap: Option<Res<Minimap>>,
    mut frames: Query<&mut UiTransform, (With<MinimapFrame>, Without<MinimapNode>)>,
    mut pictures: Query<&mut UiTransform, With<MinimapNode>>,
) {
    let Some(minimap) = minimap else {
        return;
    };
    let (squash, shift, turn) = if view.diamond {
        (
            Vec2::new(1.0, DIAMOND_SQUASH),
            {
                let past = overhang(FRAME_PX, minimap.cells());
                Val2::px(-past.x, -past.y)
            },
            Rot2::radians(DIAMOND_TURN),
        )
    } else {
        (Vec2::ONE, Val2::ZERO, Rot2::IDENTITY)
    };
    for mut frame in &mut frames {
        frame.set_if_neq(UiTransform {
            scale: squash,
            translation: shift,
            ..default()
        });
    }
    for mut picture in &mut pictures {
        picture.set_if_neq(UiTransform::from_rotation(turn));
    }
}

/// Recomposes the minimap for the current tick: terrain, then the local team's
/// fog over it, then live blips, remembered buildings, and the camera's
/// viewport outline on top.
pub fn refresh_minimap(
    mut images: ResMut<Assets<Image>>,
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    selection: Res<Selection>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    ghosts: Res<Ghosts>,
    minimap: Option<ResMut<Minimap>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    entities: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            Option<&OwnerComponent>,
            Option<&HealthComponent>,
        ),
        Without<HiddenComponent>,
    >,
) {
    let Some(mut minimap) = minimap else {
        return;
    };
    let tick = session.tick();
    // The camera's own corners, taken before the guard: the outline is the one
    // layer that answers to the camera rather than to the simulation, and the
    // camera keeps moving while the tick stands still — under a pause, a seek,
    // or an exhausted replay.
    let viewport = match (windows.single(), cameras.single()) {
        (Ok(window), Ok((camera, camera_transform))) => {
            Some(input::viewport_corners(window, camera, camera_transform).map(cell_of))
        }
        _ => None,
    };
    // Entities only move on ticks, so composing more often than that would
    // redraw and re-upload the same picture. The fog reveal is a view toggle
    // between ticks, so it forces a compose of its own, as does a camera that
    // has moved since the last one.
    if minimap.composed == Some(tick) && !reveal.is_changed() && minimap.viewport == viewport {
        return;
    }
    minimap.composed = Some(tick);
    minimap.viewport = viewport;

    let (width, height) = minimap.cells();
    let local = session.local_player();

    let Minimap { base, canvas, .. } = &mut *minimap;
    canvas.restore(base);

    // Fog. Unexplored cells fall back to the void rather than to dimmed
    // terrain: what has never been seen is not known to be there at all.
    if !reveal.0 {
        for y in 0..height {
            for x in 0..width {
                match fog.visibility_to(&session, local, x, y) {
                    CellVisibility::Unexplored => canvas.put(x, y, VOID),
                    CellVisibility::Explored => {
                        if let Some(color) = base.get(x, y) {
                            canvas.put(x, y, dim(color, EXPLORED_DIM));
                        }
                    }
                    CellVisibility::Visible => {}
                }
            }
        }
    }

    // Remembered enemy buildings, under the live blips so a rediscovered
    // building's real blip wins. A revealed map shows the real entities, so the
    // memory steps aside exactly as it does in the world view.
    if !reveal.0 {
        for (origin, size) in ghosts.remembered() {
            canvas.fill(origin.0, origin.1, size.width, size.height, GHOST);
        }
    }

    // Live blips, painted in ascending significance so nothing that matters is
    // buried: strangers first, then the local team, then what is selected.
    let selected = selection.get(local);
    let mut blips: Vec<_> = entities
        .iter()
        .filter_map(|(info, location, owner, health)| {
            let (x, y) = (
                location.position.x.to_num::<u32>(),
                location.position.y.to_num::<u32>(),
            );
            if !reveal.0 && !fog.is_visible_to(&session, local, x, y) {
                return None;
            }
            let def = registry.def(info.type_id());
            let size = def.location?.size();
            let own = owner.is_some_and(|owner| owner.player() == local);
            let team = own || owner.is_some_and(|owner| session.are_allied(local, owner.player()));

            let color = if selected.contains(&info.id()) {
                SELECTED
            } else if own && hurt_recently(health, tick) {
                HURT
            } else {
                bytes_of(render::color_for(
                    owner,
                    def.resource_source.as_ref(),
                    &session,
                ))
            };
            let significance = match (team, own, selected.contains(&info.id())) {
                (_, _, true) => 3,
                (_, true, _) => 2,
                (true, _, _) => 1,
                (false, false, false) => 0,
            };
            Some((significance, x, y, size, color))
        })
        .collect();
    blips.sort_by_key(|&(significance, ..)| significance);
    for (_, x, y, size, color) in blips {
        canvas.fill(x, y, size.width, size.height, color);
    }

    // The camera's viewport, as the quad it really covers: under the diamond
    // look the visible region is a turned square, and its bounding box would
    // claim about twice the ground.
    if let Some(corners) = viewport {
        canvas.outline(corners, VIEWPORT);
    }

    if let Some(image) = images.get_mut(&minimap.image)
        && let Some(data) = image.data.as_mut()
    {
        data.copy_from_slice(minimap.canvas.bytes());
    }
}

/// Whether an entity was hurt recently enough to answer for it, blinking once a
/// second after an opening stretch of solid color.
fn hurt_recently(health: Option<&HealthComponent>, tick: u32) -> bool {
    let Some(hit) = health.and_then(|health| health.last_hit()) else {
        return false;
    };
    let since = tick.saturating_sub(hit.tick);
    since < HURT_TICKS && (since < HURT_SOLID_TICKS || (since / HURT_PHASE_TICKS).is_multiple_of(2))
}

//
// ─── Input ────────────────────────────────────────────────────────────────────
//

/// Whether a left-press that began on the minimap is still held, so a drag that
/// wanders off the widget keeps steering the view rather than dropping it.
#[derive(Resource, Default)]
pub struct Looking(bool);

/// Where the pointer sits within the widget, counted from its top-left corner —
/// the widget itself reports its corners as `-0.5` and `0.5`.
fn cursor_within(node: &Query<&RelativeCursorPosition, With<MinimapNode>>) -> Option<Vec2> {
    Some(node.single().ok()?.normalized? + Vec2::splat(0.5))
}

/// The cell under the cursor, only while the cursor is actually over the
/// minimap.
fn hovered_cell(
    minimap: &Minimap,
    node: &Query<&RelativeCursorPosition, With<MinimapNode>>,
) -> Option<(u32, u32)> {
    if !node.single().ok()?.cursor_over {
        return None;
    }
    cell_at(minimap.cells(), cursor_within(node)?)
}

/// The cell a drag in progress points at, held to the widget's nearest edge
/// once the pointer leaves it: a drag that runs off the edge keeps panning
/// toward that edge rather than stopping where the pointer crossed it.
fn dragged_cell(
    minimap: &Minimap,
    node: &Query<&RelativeCursorPosition, With<MinimapNode>>,
) -> Option<(u32, u32)> {
    // Just short of 1.0, so the far edge names the last cell rather than the
    // one past it.
    let within = cursor_within(node)?.clamp(Vec2::ZERO, Vec2::splat(0.999_999));
    cell_at(minimap.cells(), within)
}

/// Looks where the minimap is clicked, and keeps looking while the button is
/// held so a drag pans the view. Steering the view is watching rather than
/// commanding, so this stays live during replay playback.
pub fn look_input(
    mouse: Res<ButtonInput<MouseButton>>,
    map: Res<Map>,
    minimap: Option<Res<Minimap>>,
    mut looking: ResMut<Looking>,
    node: Query<&RelativeCursorPosition, With<MinimapNode>>,
    mut cameras: Query<&mut Transform, With<Camera2d>>,
) {
    let Some(minimap) = minimap else {
        return;
    };
    if !mouse.pressed(MouseButton::Left) {
        looking.0 = false;
        return;
    }
    // Only a press that started on the widget takes hold of the view; one that
    // started on the map and dragged over the minimap keeps selecting.
    if mouse.just_pressed(MouseButton::Left) {
        looking.0 = hovered_cell(&minimap, &node).is_some();
    }
    if !looking.0 {
        return;
    }
    let Some(cell) = dragged_cell(&minimap, &node) else {
        return;
    };
    let Ok(mut transform) = cameras.single_mut() else {
        return;
    };
    let center = cell_center(cell);
    transform.translation = camera::clamp_to_map(center.extend(transform.translation.z), &map);
}

/// Orders the selection to the cell the minimap was right-clicked on. One pixel
/// is one cell, far too coarse to aim at an entity, so the click always carries
/// a position — never a target.
pub fn order_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<input::InputMode>,
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    minimap: Option<Res<Minimap>>,
    mut pending: ResMut<PendingInput>,
    node: Query<&RelativeCursorPosition, With<MinimapNode>>,
    rally_holders: Query<(&EntityInfoComponent, &OwnerComponent), With<RallyPointComponent>>,
) {
    // An armed order or placement wants a cell the player can actually see;
    // the minimap is too coarse to aim one.
    if !mouse.just_pressed(MouseButton::Right) || !matches!(*mode, input::InputMode::Normal) {
        return;
    }
    let Some(minimap) = minimap else {
        return;
    };
    let Some(cell) = hovered_cell(&minimap, &node) else {
        return;
    };
    let flush = !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));
    input::issue_orders_at(
        cell_center(cell),
        None,
        flush,
        &session,
        &selection,
        &registry,
        &rally_holders,
        &mut pending,
    );
}
