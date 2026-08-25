//! Where the renderer must and must not interpolate: an entity set back down on
//! the map has to appear where it was put rather than glide there from wherever
//! it left, while a sprite whose look changes turns to it at a rate rather than
//! snapping round.

mod utils;

use std::{f32::consts::FRAC_PI_2, time::Duration};

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_demo::render::{self, PrevPos};
use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};

use ferrets_simulation::{
    components::{hidden::HiddenComponent, location::LocationComponent},
    spawn,
};

#[test]
fn reveal_snaps_interpolation_anchor_to_where_entity_reappeared() {
    // A worker stepping out of a mine is set down beside the cell it went in
    // by — closer than a real step, so only the reveal itself tells them apart.
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    let entered = anchor_of(&app, worker);

    off_map_and_back(&mut app, worker, "21");
    app.world_mut()
        .run_system_once(render::snap_revealed)
        .expect("the reveal snaps");

    let reappeared = anchor_of(&app, worker);
    assert_eq!(
        reappeared.x,
        entered.x + render::CELL_PX,
        "the anchor must sit on the cell the worker was set down on, not {entered:?}"
    );
}

#[test]
fn walking_keeps_its_interpolation_anchor() {
    // The snap is for reappearing only: an ordinary step still interpolates, or
    // motion turns into a slideshow.
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    let before = anchor_of(&app, worker);

    set_position(&mut app, worker, "20.5");
    app.world_mut()
        .run_system_once(render::snap_revealed)
        .expect("nothing was revealed");

    assert_eq!(anchor_of(&app, worker), before);
}

/// The look is set down with the entity, for the same reason the anchor is:
/// easing into it would have a worker swing round as it steps out of a mine.
#[test]
fn reveal_snaps_look_to_where_entity_reappeared() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    face(&mut app, worker, "-1");
    // Settled looking one way, so anything but a snap leaves it looking there.
    for _ in 0..60 {
        advance_frame(&mut app);
    }
    assert!(nose_of(rotation_of(&app, worker)).distance(WEST) < 1e-6);

    app.world_mut().entity_mut(worker).insert(HiddenComponent);
    face(&mut app, worker, "1");
    app.world_mut()
        .entity_mut(worker)
        .remove::<HiddenComponent>();
    app.world_mut()
        .run_system_once(render::snap_revealed)
        .expect("the reveal snaps");

    let nose = nose_of(rotation_of(&app, worker));
    assert!(
        nose.distance(EAST) < 1e-6,
        "reappeared looking {nose} rather than east"
    );
}

//
// ─── Turning ──────────────────────────────────────────────────────────────────
//

/// A look within reach of one frame's turn is taken up whole, so a sprite that
/// has caught up with its walk stops turning and sits exactly on it.
#[test]
fn look_within_reach_is_taken_up_whole() {
    let ahead = Quat::from_rotation_z(0.1);
    assert_eq!(render::turn_toward(Quat::IDENTITY, ahead, 0.5), ahead);
    assert_eq!(render::turn_toward(ahead, ahead, 0.0), ahead);
}

/// A look further off than that is approached, not reached: the sprite turns by
/// the frame's allowance and no more, whatever the walk did.
#[test]
fn look_beyond_reach_is_turned_toward_by_allowance() {
    let allowance = 0.25;
    let turned = render::turn_toward(Quat::IDENTITY, Quat::from_rotation_z(FRAC_PI_2), allowance);

    // Screen-space rotation is f32 and the turn runs through a slerp, so the
    // angle lands on the allowance to within the arithmetic rather than on it.
    let turn = turned.angle_between(Quat::IDENTITY);
    assert!(
        (turn - allowance).abs() < 1e-5,
        "turned {turn} where {allowance} was allowed"
    );
    assert_ne!(turned, Quat::from_rotation_z(FRAC_PI_2));
}

/// The interpolation is what spends the allowance: the sprite turns a frame at a
/// time and settles looking where the walk does.
#[test]
fn drawing_turns_sprite_toward_its_look() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    // Looking back the way it came, which is the longest turn there is.
    face(&mut app, worker, "-1");
    let looking = rotation_of(&app, worker);

    advance_frame(&mut app);
    let turning = rotation_of(&app, worker);
    assert_ne!(turning, looking, "the sprite must turn");
    assert_ne!(nose_of(turning), WEST, "and not arrive in one frame");

    // A whole second at the turn rate covers a half turn several times over, so
    // the turn is spent and the sprite holds still from then on.
    for _ in 0..60 {
        advance_frame(&mut app);
    }
    let settled = rotation_of(&app, worker);
    advance_frame(&mut app);
    assert_eq!(rotation_of(&app, worker), settled, "the turn must settle");

    // Where the nose ends up, rather than which of the two quaternions says so:
    // a rotation has both, and screen-space trigonometry is f32.
    let nose = nose_of(settled);
    assert!(
        nose.distance(WEST) < 1e-6,
        "settled looking {nose} rather than west"
    );
}

/// Switched off, the drawing hands the sprite the tick's own position and look
/// rather than a way toward them — twenty jumps a second, which is what a walk
/// looks like when nothing smooths it.
#[test]
fn unsmoothed_drawing_puts_sprite_on_tick_itself() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    app.world_mut().insert_resource(render::Smoothing(false));
    face(&mut app, worker, "-1");

    // Mid-tick, where a smoothed sprite would be drawn part way from the last
    // position and part way round toward the new look.
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .advance_by(Duration::from_nanos(25_000_000));
    set_position(&mut app, worker, "21");
    advance_frame(&mut app);

    assert_eq!(
        app.world().get::<Transform>(worker).unwrap().translation,
        anchor_of(&app, worker) + Vec3::X * render::CELL_PX,
        "the sprite must stand on the cell the tick put it on"
    );
    let nose = nose_of(rotation_of(&app, worker));
    assert!(
        nose.distance(WEST) < 1e-6,
        "and look where the tick looks, not {nose} on the way there"
    );
}

/// The look the sprite is drawn at is what a direction cue has to agree with:
/// taken from the simulation instead, the line snaps a whole tick ahead of the
/// shape it belongs to.
#[test]
fn facing_line_follows_drawn_look_not_tick() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    face(&mut app, worker, "-1");
    advance_frame(&mut app);

    // Part way through the turn, so the drawn look and the tick's own differ.
    let drawn = nose_of(rotation_of(&app, worker));
    assert_ne!(drawn, WEST, "the sprite must still be turning");
    assert_eq!(
        render::facing_line(&app.world().get::<Transform>(worker).unwrap().rotation),
        drawn,
        "the line must point where the sprite does"
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Where a sprite looking along the sim's `-x` is drawn pointing: the demo draws
/// the sim's `+y` downward, so the two axes agree on west.
const WEST: Vec2 = Vec2::NEG_X;

/// And the other way, for a look set down facing back.
const EAST: Vec2 = Vec2::X;

/// A drawn peasant at `x` on row 20, carrying the render components a drawn
/// entity has.
fn spawn_worker(app: &mut App, x: &str) -> Entity {
    let (worker, _) = spawn::spawn_entity(
        app.world_mut(),
        "peasant",
        utils::part_way(x, "20"),
        Some(0),
    )
    .expect("peasant spawns");
    app.world_mut()
        .run_system_once(render::attach_sprites)
        .expect("sprites attach");
    // What the renderer's own plugins require alongside a mesh, and a headless
    // app has none of them to do it.
    app.world_mut()
        .entity_mut(worker)
        .insert(Visibility::default());
    worker
}

/// Where the entity's interpolation starts from.
fn anchor_of(app: &App, entity: Entity) -> Vec3 {
    app.world()
        .get::<PrevPos>(entity)
        .expect("a drawn entity carries an interpolation anchor")
        .anchor()
}

/// Moves the entity to `x` on row 20, as the simulation would.
fn set_position(app: &mut App, entity: Entity, x: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = utils::part_way(x, "20");
}

/// Takes the entity off the map and sets it back down at `x`, as a reveal does.
fn off_map_and_back(app: &mut App, entity: Entity, x: &str) {
    app.world_mut().entity_mut(entity).insert(HiddenComponent);
    set_position(app, entity, x);
    app.world_mut()
        .entity_mut(entity)
        .remove::<HiddenComponent>();
}

/// Points `entity`'s look along the x axis, `x` being its direction.
fn face(app: &mut App, entity: Entity, x: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .facing = FixedVec2::new(utils::signed_cells(x), FixedI64::ZERO);
}

/// The rotation the sprite is drawn at.
fn rotation_of(app: &App, entity: Entity) -> Quat {
    app.world().get::<Transform>(entity).unwrap().rotation
}

/// Runs a frame of drawing a sixtieth of a second long.
fn advance_frame(app: &mut App) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_nanos(16_666_667));
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the sprites are drawn");
}

/// Which way the drawn shape's nose points, a shape pointing `+Y` at rest.
fn nose_of(rotation: Quat) -> Vec2 {
    (rotation * Vec3::Y).truncate()
}
