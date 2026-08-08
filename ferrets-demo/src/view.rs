//! The world's on-screen look. The diamond is presentation only: the camera
//! turns the square world 45° and halves its height, so every sprite, gizmo,
//! and cursor pick converts through the one camera transform — nothing else
//! in the demo knows which look is active.

use bevy::prelude::*;

use crate::settings::Settings;

/// The active look, mirrored from the menu's choice.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct WorldView {
    /// Whether the world draws as diamonds.
    pub diamond: bool,
}

/// Keeps the look in step with the menu's choice.
pub fn sync_view(settings: Res<Settings>, mut view: ResMut<WorldView>) {
    view.set_if_neq(WorldView {
        diamond: settings.view.diamond(),
    });
}

/// Points the camera per the active look: the diamond view turns the world
/// 45° and shows twice the height (squashing it 2:1 on screen), the square
/// view looks straight down. Only rotation and scale — panning owns the
/// translation.
pub fn apply_view(view: Res<WorldView>, mut cameras: Query<&mut Transform, With<Camera2d>>) {
    let (rotation, scale) = if view.diamond {
        (
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
            Vec3::new(1.0, 2.0, 1.0),
        )
    } else {
        (Quat::IDENTITY, Vec3::ONE)
    };
    for mut transform in &mut cameras {
        if transform.rotation != rotation || transform.scale != scale {
            transform.rotation = rotation;
            transform.scale = scale;
        }
    }
}
