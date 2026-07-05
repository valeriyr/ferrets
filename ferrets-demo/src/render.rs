//! Renders simulation entities as placeholder colored shapes: triangles for
//! combat units, circles for workers, and squares for buildings and resources.
//!
//! Render components are attached directly to the simulation entities, so they
//! despawn automatically with them. Positions are interpolated between the
//! previous and current tick against the fixed-step overstep, so motion is
//! smooth and stays locked to the simulation cadence (it can never outrun it).
//! Unit shapes rotate to point in their facing direction.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use ferrets_math::{fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        location::{LocationComponent, LocationStaticData},
        owner::OwnerComponent,
        resource::ResourceSourceStaticData,
    },
    map::Map,
    selection::Selection,
    session::GameSession,
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

/// World-space center of a footprint, in pixels (Bevy y points up, sim y down).
fn world_center(position: FixedUVec2, size: NavSize) -> Vec3 {
    let cx = position.x.to_num::<f32>() + size.width as f32 / 2.0;
    let cy = position.y.to_num::<f32>() + size.height as f32 / 2.0;
    Vec3::new(cx * CELL_PX, -cy * CELL_PX, 1.0)
}

fn color_for(owner: Option<&OwnerComponent>, source: Option<&ResourceSourceStaticData>) -> Color {
    // Resource sources are colored by what they yield, regardless of ownership.
    if let Some(source) = source {
        return match source.kind() {
            "wood" => Color::srgb(0.45, 0.30, 0.15), // tree — brown
            "gold" => Color::srgb(0.85, 0.7, 0.2),   // gold mine — yellow
            _ => Color::srgb(0.75, 0.7, 0.4),        // other source — tan
        };
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
    /// Barracks — a hexagon, distinct from the main hall.
    Hexagon,
    /// Main buildings and resource sources — a square.
    Square,
}

/// Picks a shape from the entity type name. Add new types here.
fn shape_for(type_name: &str) -> Shape {
    match type_name {
        "archer" | "grunt" => Shape::Triangle,
        "peasant" | "peon" => Shape::Circle,
        "barracks" | "orc_barracks" => Shape::Hexagon,
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
        let color = color_for(owner, source);
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
            Shape::Hexagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 6))),
                    MeshMaterial2d(materials.add(color)),
                ));
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
    mut query: Query<(
        &LocationComponent,
        &LocationStaticData,
        &PrevPos,
        &mut Transform,
        &mut Visibility,
        Option<&HiddenComponent>,
        Option<&Directional>,
    )>,
) {
    let alpha = fixed.overstep_fraction().clamp(0.0, 1.0);
    for (location, location_data, prev, mut transform, mut visibility, hidden, directional) in
        &mut query
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
        *visibility = if hidden.is_some() {
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
        (&EntityInfoComponent, &LocationStaticData, &Transform),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    let selected = selection.get(session.local_player());
    if selected.is_empty() {
        return;
    }
    for (info, location_data, transform) in &query {
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

/// Draws a short line from each unit's center in its facing direction (Update).
pub fn draw_facing(
    mut gizmos: Gizmos,
    query: Query<
        (&LocationComponent, &LocationStaticData, &Transform),
        (With<Directional>, Without<HiddenComponent>),
    >,
) {
    for (location, location_data, transform) in &query {
        let Some(dir) = facing_dir(location.facing) else {
            continue;
        };
        let size = location_data.size();
        let length = size.width.min(size.height) as f32 * CELL_PX * 0.6;
        let center = transform.translation.truncate();
        gizmos.line_2d(center, center + dir * length, Color::srgb(1.0, 1.0, 0.4));
    }
}

/// Draws a faint grid over the playable area, tinting occupied ground cells
/// (run in `Update`), when enabled.
pub fn draw_grid(mut gizmos: Gizmos, map: Res<Map>, debug: Res<crate::debug::DebugState>) {
    if !debug.grid {
        return;
    }
    let (w, h) = (map.width() as f32, map.height() as f32);
    let line = Color::srgba(0.0, 0.0, 0.0, 0.15);

    // Fill occupied cells so the nav grid's occupancy is visible at a glance.
    let nav_grid = map.nav_grid();
    for y in 0..map.height() {
        for x in 0..map.width() {
            if nav_grid.is_occupied(crate::map::GROUND, NavPos::new(x, y)) {
                fill_cell(&mut gizmos, x, y);
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
