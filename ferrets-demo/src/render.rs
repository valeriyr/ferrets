//! Renders simulation entities as placeholder colored shapes: triangles for
//! combat units, circles for workers, and squares for buildings and resources.
//!
//! Render components are attached directly to the simulation entities, so they
//! despawn automatically with them. Positions are interpolated between the
//! previous and current tick against the fixed-step overstep, so motion is
//! smooth and stays locked to the simulation cadence (it can never outrun it).
//! Unit shapes rotate to point in their facing direction.

use std::{
    collections::{HashMap, HashSet},
    f32::consts::FRAC_PI_2,
};

use bevy::prelude::*;

use crate::{map, scenario::CurrentScenario, states::InGameUi};
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        location::{LocationComponent, LocationStaticData},
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        resource::ResourceSourceStaticData,
        tags::{self, TagsComponent},
    },
    selection::Selection,
    session::GameSession,
    simulation_id::SimulationId,
    visibility::{CellVisibility, VisibilityGrid},
};

/// Screen pixels per grid cell.
pub const CELL_PX: f32 = 32.0;

/// The interpolated render position from the previous tick.
#[derive(Component)]
pub struct PrevPos(Vec3);

/// Marks an entity that already has its render components attached.
#[derive(Component)]
pub struct Renderable;

/// Marks a renderable whose shape rotates to point in its facing direction
/// (mobile units; buildings and resources stay axis-aligned).
#[derive(Component)]
pub struct Directional;

/// A per-cell fog overlay sprite, darkened by the local team's visibility.
#[derive(Component)]
pub struct FogTile {
    x: u32,
    y: u32,
}

/// The outline a ghost redraws, matching the shape its building renders as (see
/// [`shape_for`]).
enum GhostShape {
    /// A rectangle spanning the footprint — square buildings.
    Rect { extent: Vec2 },
    /// A regular polygon of `sides` — the hexagon barracks and octagon fortress.
    Polygon { sides: u32, circumradius: f32 },
}

/// The last-seen appearance of a scouted enemy building, kept so it can be drawn
/// as a dimmed ghost while its cell is remembered but out of sight. Render-only
/// and local to this client; persists (stale) even after the real building dies
/// in the fog, until the cell is seen again.
struct GhostSprite {
    origin: (u32, u32),
    center: Vec2,
    shape: GhostShape,
}

/// Last-seen enemy buildings, keyed by [`SimulationId`] (see [`GhostSprite`]).
#[derive(Resource, Default)]
pub struct Ghosts(HashMap<SimulationId, GhostSprite>);

/// When set, the local view reveals the whole map — the fog overlay clears and
/// fogged entities draw — for inspecting the game. A presentation-only toggle;
/// the simulation and AI still respect fog, so it cannot cause a desync.
#[derive(Resource, Default)]
pub struct FogReveal(pub bool);

/// Toggles the map-reveal view (see [`FogReveal`]) on the `V` key.
pub fn toggle_fog_reveal(keys: Res<ButtonInput<KeyCode>>, mut reveal: ResMut<FogReveal>) {
    if keys.just_pressed(KeyCode::KeyV) {
        reveal.0 = !reveal.0;
    }
}

/// World-space center of a footprint, in pixels (Bevy y points up, sim y down).
pub(crate) fn world_center(position: FixedUVec2, size: NavSize) -> Vec3 {
    let cx = position.x.to_num::<f32>() + size.width as f32 / 2.0;
    let cy = position.y.to_num::<f32>() + size.height as f32 / 2.0;
    Vec3::new(cx * CELL_PX, -cy * CELL_PX, 1.0)
}

fn color_for(
    owner: Option<&OwnerComponent>,
    source: Option<&ResourceSourceStaticData>,
    session: &GameSession,
) -> Color {
    // Resource sources are colored by what they yield, regardless of ownership.
    if let Some(source) = source {
        return match source.kind() {
            "wood" => Color::srgb(0.45, 0.30, 0.15), // tree — brown
            "gold" => Color::srgb(0.85, 0.7, 0.2),   // gold mine — yellow
            _ => Color::srgb(0.75, 0.7, 0.4),        // other source — tan
        };
    }
    // Environment combatants share one color, whichever slot seats them.
    if owner.is_some_and(|owner| session.is_environment_slot(owner.player())) {
        return Color::srgb(0.2, 0.22, 0.26); // dark slate
    }
    match owner.map(|o| o.player()) {
        Some(0) => Color::srgb(0.35, 0.55, 1.0), // player 0 — blue
        Some(1) => Color::srgb(1.0, 0.35, 0.35), // player 1 — red
        Some(2) => Color::srgb(0.4, 0.8, 0.4),   // player 2 — green
        Some(3) => Color::srgb(0.7, 0.4, 0.9),   // player 3 — purple
        _ => Color::srgb(0.75, 0.7, 0.4),        // neutral — tan
    }
}

/// The placeholder shape used for an entity type.
enum Shape {
    /// Combat units — a triangle that points where it faces.
    Triangle,
    /// Workers — a circle.
    Circle,
    /// Ships — a hull with a triangular bow pointing where it faces.
    Ship,
    /// Barracks — a hexagon, distinct from the main hall.
    Hexagon,
    /// The sea fortress — an octagon with an inner keep.
    Fortress,
    /// Main buildings and resource sources — a square.
    Square,
}

/// Picks a shape from the entity type name. Add new types here.
fn shape_for(type_name: &str) -> Shape {
    match type_name {
        "archer" | "grunt" => Shape::Triangle,
        "peasant" | "peon" => Shape::Circle,
        "ship" => Shape::Ship,
        "barracks" | "orc_barracks" => Shape::Hexagon,
        "sea_fortress" => Shape::Fortress,
        _ => Shape::Square,
    }
}

/// Attaches a placeholder shape to any simulation entity that lacks one.
///
/// The shape is chosen from the entity type name (see [`shape_for`]). Units (the
/// non-square shapes) also get a [`Directional`] marker so they rotate to face
/// their look direction.
pub fn attach_sprites(
    mut commands: Commands,
    session: Res<GameSession>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<
        (
            Entity,
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
            Option<&OwnerComponent>,
            Option<&ResourceSourceStaticData>,
        ),
        Without<Renderable>,
    >,
) {
    for (entity, info, location, location_data, owner, source) in &query {
        let size = location_data.size();
        let center = world_center(location.position, size);
        let color = color_for(owner, source, &session);
        let radius = size.width.min(size.height) as f32 * CELL_PX * 0.45;

        let mut entity = commands.entity(entity);
        match shape_for(info.type_name()) {
            Shape::Triangle => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 3))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Circle => {
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Ship => {
                // A hull with a triangular bow whose base matches the hull's
                // width, sitting flush on the forward (+Y at rest) edge; the
                // whole silhouette rotates with the facing.
                let hull = Vec2::new(0.55, 0.65) * CELL_PX;
                // An equilateral triangle with side `s` has circumradius
                // `s/√3` and inradius `s/(2√3)` — the base offset below its
                // center.
                let circumradius = hull.x / 3f32.sqrt();
                let bow = meshes.add(RegularPolygon::new(circumradius, 3));
                let material = materials.add(color);
                entity.insert((Sprite::from_color(color, hull), Directional));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(bow),
                        MeshMaterial2d(material),
                        Transform::from_translation(Vec3::new(
                            0.0,
                            hull.y / 2.0 + circumradius / 2.0,
                            0.1,
                        )),
                    ));
                });
            }
            Shape::Hexagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 6))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Fortress => {
                // An octagonal wall with a lighter inner keep.
                let keep = meshes.add(RegularPolygon::new(radius * 0.45, 8));
                let keep_color = materials.add(color.lighter(0.12));
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 8))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(keep),
                        MeshMaterial2d(keep_color),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Square => {
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
            }
        }
        entity.insert((
            Transform::from_translation(center),
            PrevPos(center),
            Renderable,
        ));
    }
}

/// The facing direction as a normalized world-space vector, or `None` when there
/// is no facing yet. Sim y points down; Bevy y points up.
fn facing_dir(facing: FixedVec2) -> Option<Vec2> {
    if facing == FixedVec2::ZERO {
        return None;
    }
    Vec2::new(facing.x.to_num::<f32>(), -facing.y.to_num::<f32>()).try_normalize()
}

/// Converts a sim facing direction into a Z rotation for a shape that points
/// `+Y` at rest, or `None` when there is no facing yet (so rotation is left as is).
fn facing_rotation(facing: FixedVec2) -> Option<Quat> {
    let dir = facing_dir(facing)?;
    Some(Quat::from_rotation_z(dir.y.atan2(dir.x) - FRAC_PI_2))
}

/// Snapshots each sprite's current sim position as the interpolation start, run
/// before the simulation advances (`FixedPreUpdate`).
pub fn record_prev(mut query: Query<(&LocationComponent, &LocationStaticData, &mut PrevPos)>) {
    for (location, location_data, mut prev) in &mut query {
        prev.0 = world_center(location.position, location_data.size());
    }
}

/// Interpolates each sprite between its previous and current sim position by the
/// fixed-step overstep, and hides off-map entities (run in `Update`).
pub fn interpolate_sprites(
    fixed: Res<Time<Fixed>>,
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut query: Query<(
        &LocationComponent,
        &LocationStaticData,
        &PrevPos,
        &mut Transform,
        &mut Visibility,
        Option<&HiddenComponent>,
        Option<&OwnerComponent>,
        Option<&Directional>,
    )>,
) {
    let alpha = fixed.overstep_fraction().clamp(0.0, 1.0);
    let local = session.local_player();
    for (
        location,
        location_data,
        prev,
        mut transform,
        mut visibility,
        hidden,
        owner,
        directional,
    ) in &mut query
    {
        let curr = world_center(location.position, location_data.size());
        // Snap rather than slide across teleports/reveals.
        transform.translation = if prev.0.distance(curr) > 1.5 * CELL_PX {
            curr
        } else {
            prev.0.lerp(curr, alpha)
        };
        if directional.is_some()
            && let Some(rotation) = facing_rotation(location.facing)
        {
            transform.rotation = rotation;
        }
        // Own and allied entities always draw; an enemy or neutral one is hidden
        // while its cell is not in the local team's vision (fog of war). Building
        // ghosts (draw_ghosts) stand in for last-seen enemy structures.
        let fogged = !reveal.0
            && match owner {
                Some(owner)
                    if owner.player() == local || session.are_allied(local, owner.player()) =>
                {
                    false
                }
                _ => !fog.is_visible_to(
                    &session,
                    local,
                    location.position.x.to_num::<u32>(),
                    location.position.y.to_num::<u32>(),
                ),
            };
        *visibility = if hidden.is_some() || fogged {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

/// Draws a ring around the local player's selected entities (run in `Update`).
pub fn draw_selection(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    selection: Res<Selection>,
    query: Query<
        (
            &EntityInfoComponent,
            &LocationStaticData,
            &Transform,
            &Visibility,
        ),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    let selected = selection.get(session.local_player());
    if selected.is_empty() {
        return;
    }
    for (info, location_data, transform, visibility) in &query {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        if selected.contains(&info.id()) {
            let size = location_data.size();
            // Larger than the sprite so the ring isn't hidden behind it.
            let radius = size.width.max(size.height) as f32 * CELL_PX * 0.7;
            gizmos.circle_2d(
                transform.translation.truncate(),
                radius,
                Color::srgb(0.2, 1.0, 0.4),
            );
        }
    }
}

/// Draws the rally point of each selected own producer: a line from the
/// building to the target and a circle marking it (run in `Update`).
pub fn draw_rally(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    selection: Res<Selection>,
    holders: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &LocationStaticData,
        &OwnerComponent,
        &RallyPointComponent,
    )>,
    targets: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
        ),
        Without<HiddenComponent>,
    >,
) {
    const COLOR: Color = Color::srgb(1.0, 0.65, 0.2);

    let local = session.local_player();
    for (info, location, location_data, owner, rally) in &holders {
        if owner.player() != local || !selection.get(local).contains(&info.id()) {
            continue;
        }
        let Some(target) = rally.0 else {
            continue;
        };
        let end = match target {
            // The move lands in the cell containing the position, so mark that
            // cell's center rather than the raw sub-cell position.
            RallyTarget::Position(position) => {
                world_center(FixedUVec2::from(NavPos::from(position)), NavSize::ONE)
            }
            RallyTarget::Entity(id) => {
                // A vanished target leaves nothing to point at.
                let Some((_, target_location, target_data)) = targets
                    .iter()
                    .find(|(target_info, ..)| target_info.id() == id)
                else {
                    continue;
                };
                world_center(target_location.position, target_data.size())
            }
        };
        let start = world_center(location.position, location_data.size());
        gizmos.line_2d(start.truncate(), end.truncate(), COLOR);
        gizmos.circle_2d(end.truncate(), CELL_PX * 0.3, COLOR);
    }
}

/// Draws a short line from each unit's center in its facing direction (Update).
pub fn draw_facing(
    mut gizmos: Gizmos,
    query: Query<
        (
            &LocationComponent,
            &LocationStaticData,
            &Transform,
            &Visibility,
        ),
        (With<Directional>, Without<HiddenComponent>),
    >,
) {
    for (location, location_data, transform, visibility) in &query {
        // Don't trace a unit hidden by fog (interpolate_sprites set its visibility).
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(dir) = facing_dir(location.facing) else {
            continue;
        };
        let size = location_data.size();
        let length = size.width.min(size.height) as f32 * CELL_PX * 0.6;
        let center = transform.translation.truncate();
        gizmos.line_2d(center, center + dir * length, Color::srgb(1.0, 1.0, 0.4));
    }
}

/// The tile color for a terrain, by content name.
fn terrain_color(terrain: &str) -> Color {
    match terrain {
        "grass" => Color::srgb(0.20, 0.34, 0.17),
        "water" => Color::srgb(0.15, 0.35, 0.6),
        _ => Color::srgb(0.35, 0.33, 0.30), // unknown terrain — bare dirt
    }
}

/// Spawns a colored tile per cell from the map's terrain, so the whole
/// playable field is drawn and the void outside it stays the clear color.
/// Terrain is static, so the tiles spawn once on entering the game; they
/// despawn with the in-game overlay on teardown.
pub fn spawn_terrain_tiles(
    mut commands: Commands,
    session: Res<GameSession>,
    scenario: Option<Res<CurrentScenario>>,
) {
    let map = match &scenario {
        Some(scenario) => scenario.0.map.clone(),
        None => match map::by_name(session.map()) {
            Some(map) => map,
            None => return,
        },
    };

    for (i, &terrain) in map.terrain_cells().iter().enumerate() {
        let Some(name) = map.terrains().get(terrain as usize) else {
            continue;
        };
        let (x, y) = (i as u32 % map.width(), i as u32 / map.width());
        let center = Vec3::new((x as f32 + 0.5) * CELL_PX, -(y as f32 + 0.5) * CELL_PX, 0.0);
        commands.spawn((
            Sprite::from_color(terrain_color(name), Vec2::splat(CELL_PX)),
            Transform::from_translation(center),
            InGameUi,
        ));
    }

    // Fog overlay: one darkening tile per cell above the terrain but below
    // entities (z 0.5), updated each frame from the local team's visibility.
    for y in 0..map.height() {
        for x in 0..map.width() {
            let center = Vec3::new((x as f32 + 0.5) * CELL_PX, -(y as f32 + 0.5) * CELL_PX, 0.5);
            commands.spawn((
                FogTile { x, y },
                Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 1.0), Vec2::splat(CELL_PX)),
                Transform::from_translation(center),
                InGameUi,
            ));
        }
    }
}

/// Darkens each fog tile by the local team's knowledge of its cell: black when
/// unexplored, dimmed when explored-but-unseen, clear when visible.
pub fn update_fog_overlay(
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut tiles: Query<(&FogTile, &mut Sprite)>,
) {
    let local = session.local_player();
    for (tile, mut sprite) in &mut tiles {
        let alpha = if reveal.0 {
            0.0
        } else {
            match fog.visibility_to(&session, local, tile.x, tile.y) {
                CellVisibility::Unexplored => 1.0,
                CellVisibility::Explored => 0.55,
                CellVisibility::Visible => 0.0,
            }
        };
        sprite.color = Color::srgba(0.0, 0.0, 0.0, alpha);
    }
}

/// Snapshots scouted enemy buildings while visible and draws their last-seen
/// ghost while their cell is remembered but out of sight; purges a ghost once
/// its cell is seen again and the real building is gone.
pub fn draw_ghosts(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut ghosts: ResMut<Ghosts>,
    buildings: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &LocationStaticData,
            &TagsComponent,
            Option<&OwnerComponent>,
        ),
        Without<HiddenComponent>,
    >,
) {
    let local = session.local_player();
    let mut alive = HashSet::new();
    for (info, location, location_data, tags, owner) in &buildings {
        let own_team =
            owner.is_some_and(|o| o.player() == local || session.are_allied(local, o.player()));
        if own_team || !tags.contains(tags::BUILDING) {
            continue;
        }
        alive.insert(info.id());
        let (x, y) = (
            location.position.x.to_num::<u32>(),
            location.position.y.to_num::<u32>(),
        );
        if fog.is_visible_to(&session, local, x, y) {
            let size = location_data.size();
            ghosts.0.insert(
                info.id(),
                GhostSprite {
                    origin: (x, y),
                    center: world_center(location.position, size).truncate(),
                    shape: ghost_shape(info.type_name(), size),
                },
            );
        }
    }

    ghosts.0.retain(|id, ghost| {
        match fog.visibility_to(&session, local, ghost.origin.0, ghost.origin.1) {
            // Seen again: keep only if the building is still there (else it was
            // destroyed while we were away, so drop the stale ghost).
            CellVisibility::Visible => alive.contains(id),
            // Remembered but unseen: draw the ghost in its last-known place
            // (unless the whole map is revealed, when the real entities show).
            CellVisibility::Explored => {
                if !reveal.0 {
                    let iso = Isometry2d::from_translation(ghost.center);
                    let color = Color::srgba(0.55, 0.55, 0.62, 0.6);
                    match &ghost.shape {
                        GhostShape::Rect { extent } => gizmos.rect_2d(iso, *extent, color),
                        GhostShape::Polygon {
                            sides,
                            circumradius,
                        } => {
                            gizmos.primitive_2d(
                                &RegularPolygon::new(*circumradius, *sides),
                                iso,
                                color,
                            );
                        }
                    }
                }
                true
            }
            CellVisibility::Unexplored => false,
        }
    });
}

/// The ghost outline for a building of `type_name` occupying `size` cells —
/// matched to what [`attach_sprites`] draws for the live building.
fn ghost_shape(type_name: &str, size: NavSize) -> GhostShape {
    let circumradius = size.width.min(size.height) as f32 * CELL_PX * 0.45;
    match shape_for(type_name) {
        Shape::Hexagon => GhostShape::Polygon {
            sides: 6,
            circumradius,
        },
        Shape::Fortress => GhostShape::Polygon {
            sides: 8,
            circumradius,
        },
        _ => GhostShape::Rect {
            extent: Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85,
        },
    }
}
