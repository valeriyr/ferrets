//! 2D camera: spawn, WASD/arrow pan, and scroll-wheel zoom.

use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};
use ferrets_simulation::{map::Map, session::GameSession};

use crate::{render::CELL_PX, sound};

const PAN_SPEED: f32 = 700.0;
/// Closest and furthest orthographic zoom (smaller = more zoomed in).
const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 2.0;

/// Clamps a camera translation so the view stays within the map bounds (Bevy y
/// points up, the map down). Shared by pan/zoom and any programmatic recenter.
/// The camera moves in world space whatever its look, so the bounds are the
/// map's own rectangle.
pub fn clamp_to_map(translation: Vec3, map: &Map) -> Vec3 {
    let max_x = map.width() as f32 * CELL_PX;
    let min_y = -(map.height() as f32 * CELL_PX);
    Vec3::new(
        translation.x.clamp(0.0, max_x),
        translation.y.clamp(min_y, 0.0),
        translation.z,
    )
}

/// Spawns the 2D camera. It is framed on the local player's base once the game
/// starts (see [`frame_local_player`]); the local player isn't known yet here.
pub fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Transform::default(), sound::listener()));
}

/// Centers the camera on the local player's start point when the game begins.
/// Runs after the scene spawners so it reads the map the game actually opens on.
pub fn frame_local_player(
    session: Res<GameSession>,
    map: Res<Map>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    // A node with no local player — an observer's, a replay's — has no base
    // of its own to open on, so it opens on the first player's instead: a
    // watcher starts where the action starts. The middle of the map is the
    // last resort, for a lineup with no start points at all.
    let framed = session
        .local_player()
        .and_then(|local| map.start_point(local))
        .or_else(|| {
            session
                .player_slots()
                .find_map(|slot| map.start_point(slot.id()))
        });
    let (x, y) = match framed {
        Some(start) => (start.x as f32 + 5.0, start.y as f32 + 5.0),
        None => (map.width() as f32 / 2.0, map.height() as f32 / 2.0),
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    transform.translation.x = x * CELL_PX;
    transform.translation.y = -y * CELL_PX;
}

/// Pans the camera with WASD/arrows and zooms with the scroll wheel, keeping the
/// view centered within the map bounds.
pub fn pan_zoom(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    map: Res<Map>,
    mut camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if dir != Vec2::ZERO {
        // Pan in screen directions: the shift converts through the camera's
        // own orientation and scale, so W always moves the view up whatever
        // the look.
        let step = dir.normalize() * PAN_SPEED * time.delta_secs();
        let local = Vec3::new(step.x * transform.scale.x, step.y * transform.scale.y, 0.0);
        let world = transform.rotation * local;
        transform.translation += world;
    }

    transform.translation = clamp_to_map(transform.translation, &map);

    if scroll.delta.y != 0.0
        && let Projection::Orthographic(ortho) = &mut *projection
    {
        ortho.scale = (ortho.scale * (1.0 - scroll.delta.y * 0.1)).clamp(MIN_ZOOM, MAX_ZOOM);
    }
}
