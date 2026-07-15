//! 2D camera: spawn, WASD/arrow pan, and scroll-wheel zoom.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use ferrets_simulation::map::Map;
use ferrets_simulation::session::GameSession;

use crate::render::CELL_PX;

const PAN_SPEED: f32 = 700.0;
/// Closest and furthest orthographic zoom (smaller = more zoomed in).
const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 2.0;

/// Spawns the 2D camera. It is framed on the local player's base once the game
/// starts (see [`frame_local_player`]); the local player isn't known yet here.
pub fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Transform::default()));
}

/// Centers the camera on the local player's start point when the game begins.
/// Runs after the scene spawners so it reads the map the game actually opens on.
pub fn frame_local_player(
    session: Res<GameSession>,
    map: Res<Map>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Some(start) = map.start_point(session.local_player()) else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    transform.translation.x = (start.x as f32 + 5.0) * CELL_PX;
    transform.translation.y = -(start.y as f32 + 5.0) * CELL_PX;
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
        transform.translation += (dir.normalize() * PAN_SPEED * time.delta_secs()).extend(0.0);
    }

    // Keep the camera centered within the map (Bevy y points up, the map down).
    let max_x = map.width() as f32 * CELL_PX;
    let min_y = -(map.height() as f32 * CELL_PX);
    transform.translation.x = transform.translation.x.clamp(0.0, max_x);
    transform.translation.y = transform.translation.y.clamp(min_y, 0.0);

    if scroll.delta.y != 0.0
        && let Projection::Orthographic(ortho) = &mut *projection
    {
        ortho.scale = (ortho.scale * (1.0 - scroll.delta.y * 0.1)).clamp(MIN_ZOOM, MAX_ZOOM);
    }
}
