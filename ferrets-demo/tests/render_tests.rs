//! Where the renderer must and must not interpolate: an entity set back down on
//! the map has to appear where it was put rather than glide there from wherever
//! it left, while a sprite between two ticks is drawn part way from the look it
//! held to the look it has.

mod utils;

use std::f32::consts::FRAC_1_SQRT_2;

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use ferrets_content::{
    attack::{Delivery, Slain, Weapon},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool::Pool,
    registry::ContentRegistry,
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
};
use ferrets_demo::{
    render::{
        self, Directional, DrawnBearings, DrawnFacing, FogReveal, ObserverPerspective, PrevPos,
        Puffs, Sighted, SkillPulses,
    },
    ruleset,
    time::NOMINAL_TICK_HZ,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, facing::Facing};
use ferrets_simulation::{
    beside::{Beside, Lean},
    command::SkillTarget,
    components::{
        entity_info::EntityInfoComponent, hidden::Inside, location::LocationComponent,
        owner::OwnerComponent, turret::TurretsComponent,
    },
    events::{EventRecord, SimulationEvent},
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, local_role::LocalRole, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    spawn::{self, Boarding, SetDown},
    visibility::Sighting,
};

#[test]
fn glimpsed_entity_hides_body_sprite_and_shows_it_once_seen() {
    // A glimpse owes a presence and no more: its sprite carries the owner's
    // color and the type's own outline, so the body stays hidden and
    // draw_glimpses puts a neutral shape where it stands.
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");

    app.world_mut()
        .entity_mut(worker)
        .insert(Sighted(Sighting::Glimpsed));
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the interpolation runs");
    assert_eq!(
        app.world().get::<Visibility>(worker),
        Some(&Visibility::Hidden),
        "the body a glimpse would name its side and type by stays hidden"
    );

    app.world_mut()
        .entity_mut(worker)
        .insert(Sighted(Sighting::Seen));
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the interpolation runs");
    assert_eq!(
        app.world().get::<Visibility>(worker),
        Some(&Visibility::Visible),
        "and is drawn again the moment the side makes it out"
    );
}

#[test]
fn reveal_snaps_interpolation_anchor_to_where_entity_reappeared() {
    // A worker stepping out of a mine is set down beside the cell it went in
    // by — closer than a real step, so only the reveal itself tells them apart.
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    let entered = anchor_of(&app, worker);

    // Off the map and set back down one cell over, with no frame in between:
    // the renderer reads nothing of the time spent hidden, only the frame of
    // the set-down, which is what the snap keys on, not the distance.
    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    assert_eq!(
        spawn::place_back_beside(
            app.world_mut(),
            worker,
            Beside::Footprint(CellRect::new(CellPos::new(21, 20), CellSize::ONE)),
            Lean::WentIn
        ),
        SetDown::Placed,
        "the cell beside the mine is free"
    );
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
fn relocation_snaps_interpolation_anchor_to_where_entity_landed() {
    let mut app = utils::view_app();
    app.world_mut().init_resource::<render::Relocated>();
    let worker = spawn_worker(&mut app, "20");
    let before = anchor_of(&app, worker);

    // Moved one cell over: nearer than any jump the interpolation snaps on
    // its own.
    spawn::relocate(app.world_mut(), &[(worker, CellPos::new(21, 20))]);
    app.world_mut()
        .run_system_once(render::collect_relocations)
        .expect("the relocations are noted");
    // The tick retires what it announced before the frame draws.
    app.world_mut().resource_mut::<EventRecord>().clear();
    app.world_mut()
        .run_system_once(render::snap_relocated)
        .expect("the relocation snaps");

    assert_eq!(anchor_of(&app, worker).x, before.x + render::CELL_PX);
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
    face(&mut app, worker, Facing::WEST);
    record_tick(&mut app);
    draw(&mut app);
    assert!(
        nose_of(rotation_of(&app, worker)).distance(WEST) < 1e-6,
        "the sprite opens facing the way the entity faces"
    );

    // Off the map and set back down on the same cell, looking the other way:
    // the set-down is what the snap keys on, not any distance or turn.
    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    assert_eq!(
        spawn::place_back_beside(
            app.world_mut(),
            worker,
            Beside::Footprint(CellRect::new(CellPos::new(20, 20), CellSize::ONE)),
            Lean::WentIn
        ),
        SetDown::Placed,
        "the cell it went in by is free"
    );
    face(&mut app, worker, Facing::EAST);
    app.world_mut()
        .run_system_once(render::snap_revealed)
        .expect("the reveal snaps");

    // Part way through a tick, where anything left to interpolate would show.
    part_way_through_tick(&mut app);
    draw(&mut app);
    let nose = nose_of(rotation_of(&app, worker));
    assert!(
        nose.distance(EAST) < 1e-6,
        "reappeared looking {nose} rather than east"
    );
}

#[test]
fn sprite_leaves_view_when_entity_goes_off_map_and_returns_once_set_down() {
    // A seen worker steps into a mine: a body with no location has nothing to
    // draw, so its sprite leaves view the frame it goes, and is drawn again
    // where it is set back down.
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    app.world_mut()
        .entity_mut(worker)
        .insert(Sighted(Sighting::Seen));
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the interpolation runs");
    assert_eq!(
        app.world().get::<Visibility>(worker),
        Some(&Visibility::Visible),
        "a seen worker on the map is drawn"
    );

    // One frame off the map is as long as any: the sprite is judged by what
    // the entity carries this frame, not by how long it has been gone.
    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the interpolation runs");
    assert_eq!(
        app.world().get::<Visibility>(worker),
        Some(&Visibility::Hidden),
        "off the map, the sprite leaves view"
    );

    assert_eq!(
        spawn::place_back_beside(
            app.world_mut(),
            worker,
            Beside::Footprint(CellRect::new(CellPos::new(22, 20), CellSize::ONE)),
            Lean::WentIn
        ),
        SetDown::Placed,
        "the cell two over is free"
    );
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the interpolation runs");
    assert_eq!(
        app.world().get::<Visibility>(worker),
        Some(&Visibility::Visible),
        "set back down and still seen, it is drawn again"
    );
}

//
// ─── Turning ──────────────────────────────────────────────────────────────────
//

/// Between two ticks the sprite is drawn part way from the look it held to the
/// one it has. The simulation turns at the body's own rate, so the drawing has
/// only to fill the gap between its ticks — the same job it does for position.
#[test]
fn drawing_puts_sprite_part_way_between_two_ticks_looks() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    face(&mut app, worker, Facing::NORTH);
    record_tick(&mut app);
    face(&mut app, worker, Facing::EAST);

    part_way_through_tick(&mut app);
    draw(&mut app);

    // Half of the quarter turn from north to east is north-east.
    let nose = nose_of(rotation_of(&app, worker));
    let between = Vec2::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2);
    assert!(
        nose.distance(between) < 1e-3,
        "drawn looking {nose} rather than half way round at {between}"
    );
}

/// And it never runs past the tick: a whole step's worth of overstep is the
/// tick's own look, never a frame beyond it.
#[test]
fn drawn_look_never_runs_past_tick() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    face(&mut app, worker, Facing::NORTH);
    record_tick(&mut app);
    face(&mut app, worker, Facing::EAST);

    whole_tick(&mut app);
    draw(&mut app);

    let nose = nose_of(rotation_of(&app, worker));
    assert!(
        nose.distance(EAST) < 1e-6,
        "drawn looking {nose} rather than the tick's own east"
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
    face(&mut app, worker, Facing::NORTH);
    record_tick(&mut app);
    face(&mut app, worker, Facing::WEST);

    // Mid-tick, where a smoothed sprite would be drawn part way from the last
    // position and part way round toward the new look.
    part_way_through_tick(&mut app);
    set_position(&mut app, worker, "21");
    draw(&mut app);

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
/// taken from the simulation instead, the line runs a whole tick ahead of the
/// shape it belongs to.
#[test]
fn facing_line_follows_drawn_look_not_tick() {
    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    face(&mut app, worker, Facing::NORTH);
    record_tick(&mut app);
    face(&mut app, worker, Facing::EAST);
    part_way_through_tick(&mut app);
    draw(&mut app);

    let drawn = drawn_of(&app, worker);
    assert_ne!(
        drawn,
        Facing::EAST,
        "the drawn look must still be short of it"
    );
    // The line is trigonometry on the bearing and the nose is the shape's own
    // rotation, so they agree to within f32 rather than to the bit.
    let line = render::facing_line(drawn);
    let nose = nose_of(rotation_of(&app, worker));
    assert!(
        line.distance(nose) < 1e-6,
        "the line points {line} where the sprite points {nose}"
    );
}

//
// ─── What is traced ─────────────────────────────────────────────────────────────
//

/// A body that can walk is drawn along its look; a keep is not. Two looks, two
/// colours, and nothing chooses between them — which is what lets a hull and a gun
/// point different ways.
#[test]
fn body_and_gun_are_drawn_apart() {
    assert_ne!(
        render::LOOK_COLOR,
        render::BEARING_COLOR,
        "the two must differ, or the distinction says nothing"
    );

    let mut app = utils::view_app();
    let worker = spawn_worker(&mut app, "20");
    assert!(
        app.world().get::<Directional>(worker).is_some(),
        "a body that walks is drawn along its look"
    );
    assert!(
        app.world().get::<DrawnBearings>(worker).is_none(),
        "and carries no gun bearing of its own"
    );
}

/// The demo's own boss pins the other half: a fortress standing idle carries its
/// gun's drawn bearing and no body look, so its walls stay square while the gun is
/// what a player sees pointing.
#[test]
fn idle_sea_fortress_draws_its_gun_and_not_its_walls() {
    let mut app = utils::view_app();
    let fortress = spawn_at(&mut app, "sea_fortress", 46, 46);

    assert!(
        app.world().get::<DrawnBearings>(fortress).is_some(),
        "its weapon bears on its own, with no attack in flight at all"
    );
    assert!(
        app.world().get::<Directional>(fortress).is_none(),
        "and its walls are never turned"
    );
}

/// A body that walks *and* carries a gun of its own has both, each interpolated
/// from its own last tick: the hull points where it is going while the gun points
/// where it is trained. Sharing one drawn look would spin the hull with the gun.
#[test]
fn walking_gun_draws_hull_and_gun_apart() {
    let mut app = utils::view_app();
    register_gun_wagon(&mut app);
    let wagon = spawn_at(&mut app, "gun_wagon", 20, 20);

    face(&mut app, wagon, Facing::EAST);
    train(&mut app, wagon, Facing::NORTH);
    record_tick(&mut app);
    // The hull turns one way and the gun the other, within the same tick.
    face(&mut app, wagon, Facing::SOUTH);
    train(&mut app, wagon, Facing::WEST);
    part_way_through_tick(&mut app);
    draw(&mut app);

    // Half way from east to south is south-east; half way from north to west is
    // north-west. Each look filled in from its own previous bearing.
    let hull = drawn_of(&app, wagon);
    let gun = app
        .world()
        .get::<DrawnBearings>(wagon)
        .expect("a gun of its own")
        .bearings()[0];
    assert_eq!(hull, half_way(Facing::EAST, Facing::SOUTH), "hull {hull:?}");
    assert_eq!(gun, half_way(Facing::NORTH, Facing::WEST), "gun {gun:?}");

    // And the sprite follows the hull, not the gun.
    let nose = nose_of(rotation_of(&app, wagon));
    let expected = render::facing_line(hull);
    assert!(
        nose.distance(expected) < 1e-6,
        "the hull is drawn along {expected}, not {nose}"
    );
}

//
// ─── Fire ─────────────────────────────────────────────────────────────────────
//

#[test]
fn flames_count_by_footprint_width_and_hold_within_tick() {
    // Two plus the width, at most two more: three over a cell, four over a
    // footprint two or more cells wide.
    assert_eq!(render::flame_tongues(40, 7, CellSize::ONE).count(), 3);
    assert_eq!(render::flame_tongues(40, 7, CellSize::new(2, 2)).count(), 4);
    assert_eq!(render::flame_tongues(40, 7, CellSize::new(3, 3)).count(), 4);

    // The same tick draws the same fire; the next one moves it.
    let fire = |tick| render::flame_tongues(tick, 7, CellSize::new(3, 3)).collect::<Vec<_>>();
    assert_eq!(fire(40), fire(40));
    assert_ne!(fire(40), fire(41));
}

//
// ─── Sightings ────────────────────────────────────────────────────────────────
//

/// The stamp folds the whole footprint: a rival building lit at one cell is
/// made out, and the cell is the corner farthest from the one it anchors at,
/// which is the cell the rule used to read alone.
#[test]
fn refresh_sightings_stamps_footprint_lit_at_one_corner() {
    let mut app = utils::view_app();
    // Sight is a circle of eight cells around the grunt at (17, 18). The hall
    // spans (10, 10) to (12, 12): its far corner (12, 12) is √(5² + 6²) = 7.81
    // cells away and lit, while (11, 12) at √(6² + 6²) = 8.49 and (12, 11) at
    // √(5² + 7²) = 8.60 are not.
    spawn_owned_at(&mut app, "grunt", 17, 18, 0);
    let hall = spawn_owned_at(&mut app, "great_hall", 10, 10, 1);
    utils::run_ticks(&mut app, 2);

    app.world_mut()
        .run_system_once(render::refresh_sightings)
        .expect("the sighting pass runs");

    assert_eq!(
        app.world().get::<Sighted>(hall),
        Some(&Sighted(Sighting::Seen)),
        "any one lit cell of a footprint settles it"
    );
}

/// A concealed rival under sight and no detector is a presence and no more.
#[test]
fn refresh_sightings_stamps_undetected_cloak_under_sight_as_glimpsed() {
    let mut app = utils::view_app();
    // Two cells from the grunt, well inside its eight cells of sight; the
    // side fields no detector, and the templar is cloaked by its type.
    spawn_owned_at(&mut app, "grunt", 20, 20, 0);
    let templar = spawn_owned_at(&mut app, "dark_templar", 22, 20, 1);
    utils::run_ticks(&mut app, 2);

    app.world_mut()
        .run_system_once(render::refresh_sightings)
        .expect("the sighting pass runs");

    assert_eq!(
        app.world().get::<Sighted>(templar),
        Some(&Sighted(Sighting::Glimpsed)),
        "lit and undetected is glimpsed"
    );
}

//
// ─── Puffs ────────────────────────────────────────────────────────────────────
//

#[test]
fn hiding_marks_cell_entity_went_in_by() {
    // The announcement names an entity that has no location any more; the
    // marker stands where it went in.
    let mut app = utils::view_app();
    app.world_mut().init_resource::<Puffs>();
    let worker = spawn_worker(&mut app, "20");
    // A tick before the hiding notes the worker as made out.
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");

    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");

    assert_eq!(
        app.world().resource::<Puffs>().marked(),
        vec![(utils::part_way("20", "20"), CellSize::ONE)],
        "one marker, where the announcement says the worker went in"
    );
}

#[test]
fn boarding_marks_cell_passenger_vanished_from_not_holder() {
    let mut app = utils::view_app();
    app.world_mut().init_resource::<Puffs>();
    let (bunker, _) =
        utils::create_entity(app.world_mut(), "bunker", utils::at_cell(20, 20), Some(0))
            .expect("the demo content defines a bunker");
    let (grunt, _) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(24, 20), Some(0))
            .expect("the demo content defines a grunt");
    // A tick before the boarding notes the grunt as made out.
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");

    assert_eq!(
        spawn::board(app.world_mut(), grunt, bunker),
        Boarding::Boarded
    );
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");

    // The announcement carries where the grunt stood as it went, four cells
    // from the bunker it now rides in.
    assert_eq!(
        app.world().resource::<Puffs>().marked(),
        vec![(utils::at_cell(24, 20), CellSize::ONE)]
    );
}

#[test]
fn hiding_of_body_nobody_made_out_marks_nothing() {
    let mut app = utils::view_app();
    app.world_mut().init_resource::<Puffs>();
    let worker = spawn_worker(&mut app, "20");

    // No tick before the hiding noted the worker as made out.
    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");

    assert!(app.world().resource::<Puffs>().marked().is_empty());
}

#[test]
fn changing_view_forgets_marks_and_what_was_made_out() {
    let mut app = utils::view_app();
    let mut forget = view_change_clear(&mut app);
    let second = spawn_worker(&mut app, "24");
    seed_marks(&mut app);

    app.world_mut().insert_resource(FogReveal::Revealed);
    forget
        .run((), app.world_mut())
        .expect("the view-change clear runs");
    assert!(app.world().resource::<Puffs>().marked().is_empty());
    assert!(app.world().resource::<SkillPulses>().pulsed().is_empty());

    // What the old view made out is forgotten too: the second worker, hidden
    // before any tick of the new view noted it, leaves no marker.
    spawn::hide_entity(app.world_mut(), second, Inside::Nothing);
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");
    assert!(app.world().resource::<Puffs>().marked().is_empty());
}

#[test]
fn observer_watching_another_side_forgets_marks() {
    let mut app = utils::view_app_as(LocalRole::Observer);
    app.world_mut()
        .insert_resource(ObserverPerspective(Some(0)));
    let mut forget = view_change_clear(&mut app);
    seed_marks(&mut app);

    app.world_mut()
        .insert_resource(ObserverPerspective(Some(1)));
    forget
        .run((), app.world_mut())
        .expect("the view-change clear runs");

    assert!(app.world().resource::<Puffs>().marked().is_empty());
    assert!(app.world().resource::<SkillPulses>().pulsed().is_empty());
}

#[test]
fn player_pressing_watch_key_keeps_marks() {
    // A playing node views its own side whatever the watch says.
    let mut app = utils::view_app();
    let mut forget = view_change_clear(&mut app);
    seed_marks(&mut app);

    app.world_mut()
        .insert_resource(ObserverPerspective(Some(1)));
    forget
        .run((), app.world_mut())
        .expect("the view-change clear runs");

    assert_eq!(app.world().resource::<Puffs>().marked().len(), 1);
    assert_eq!(app.world().resource::<SkillPulses>().pulsed().len(), 1);
}

#[test]
fn rival_caster_in_fog_casting_at_cell_pulses_nothing() {
    let mut app = utils::view_app();
    app.world_mut().init_resource::<SkillPulses>();
    let skill = app
        .world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("the demo content defines battle focus");
    let (_, rival) =
        utils::create_entity(app.world_mut(), "grunt", utils::at_cell(60, 60), Some(1))
            .expect("the demo content defines a grunt");
    utils::run_ticks(&mut app, 1);
    app.world_mut()
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::SkillCast {
            caster: rival,
            target: SkillTarget::Position(utils::at_cell(20, 20)),
            skill,
        });

    app.world_mut()
        .run_system_once(render::collect_skill_pulses)
        .expect("the pulses are collected");

    assert!(app.world().resource::<SkillPulses>().pulsed().is_empty());
}

//
// ─── Glimpses and crews ───────────────────────────────────────────────────────
//

/// The rings of a glimpse sit on the cell the entity is over: a flier is drawn
/// lifted off it, and the lift is taken back out, so nothing about the rings
/// tells a flier from a walker.
#[test]
fn glimpse_center_is_ground_point_under_lifted_flier() {
    let mut app = utils::view_app();
    // Unsmoothed, the sprite stands on the tick's own position rather than
    // part way toward it.
    app.world_mut().insert_resource(render::Smoothing(false));
    let wraith = spawn_at(&mut app, "wraith", 20, 20);
    let grunt = spawn_at(&mut app, "grunt", 24, 24);
    draw(&mut app);

    // A one-cell body at (20, 20) is centered at (20.5, 20.5) cells, which is
    // (20.5 × 32, −20.5 × 32) = (656, −656) pixels, the demo's y pointing
    // down; the one at (24, 24) is at (24.5 × 32, −24.5 × 32) = (784, −784).
    let world = app.world();
    let registry = world.resource::<ContentRegistry>();
    let flier = registry
        .entity("wraith")
        .expect("the demo defines a wraith");
    let walker = registry.entity("grunt").expect("the demo defines a grunt");
    let lifted = world.get::<Transform>(wraith).expect("a drawn flier");
    let standing = world.get::<Transform>(grunt).expect("a drawn walker");

    assert_ne!(
        lifted.translation.y, -656.0,
        "a flier is drawn lifted off the cell it is over"
    );
    assert_eq!(
        render::glimpse_center(lifted, registry, flier),
        Vec2::new(656.0, -656.0),
        "and its glimpse is centered on the cell, the lift taken back out"
    );
    assert_eq!(standing.translation.truncate(), Vec2::new(784.0, -784.0));
    assert_eq!(
        render::glimpse_center(standing, registry, walker),
        Vec2::new(784.0, -784.0),
        "a walker's glimpse is where it is drawn"
    );
}

/// The work marker's dots count the viewed side's own crew wherever it is,
/// and a rival's crew only where the perspective makes it out; a glimpsed or
/// unseen rival is a shimmer beside the tree and not a dot on it.
#[test]
fn crew_counts_own_side_anywhere_and_rival_only_when_made_out() {
    // The demo map seats players 0 and 1 on no team, so 1 is 0's rival.
    let app = utils::view_app();
    let session = app.world().resource::<GameSession>();
    let own = OwnerComponent::new(0);
    let rival = OwnerComponent::new(1);
    let seen = Sighted(Sighting::Seen);
    let glimpsed = Sighted(Sighting::Glimpsed);
    let unseen = Sighted(Sighting::Unseen);

    // One of the viewed side's own inside a job: its body is hidden, and it
    // counts all the same.
    assert!(render::crew_counts(
        session,
        Some(0),
        Some(&own),
        &Visibility::Hidden,
        None
    ));
    // A rival seen in full counts.
    assert!(render::crew_counts(
        session,
        Some(0),
        Some(&rival),
        &Visibility::Visible,
        Some(&seen)
    ));
    // A rival only glimpsed does not: its body is hidden and it is not made out.
    assert!(!render::crew_counts(
        session,
        Some(0),
        Some(&rival),
        &Visibility::Hidden,
        Some(&glimpsed)
    ));
    // A rival unseen does not.
    assert!(!render::crew_counts(
        session,
        Some(0),
        Some(&rival),
        &Visibility::Hidden,
        Some(&unseen)
    ));
    // With no viewed player the whole map is watched, and every member counts.
    assert!(render::crew_counts(
        session,
        None,
        Some(&rival),
        &Visibility::Hidden,
        Some(&unseen)
    ));
}

#[test]
fn passenger_dot_takes_color_of_whose_passenger_is() {
    // Players 0 and 1 share a team; player 2 stands alone.
    let session = GameSession::configured(
        LocalRole::Player(0),
        vec![
            PlayerSlot::occupied(0, PlayerType::Human, Some("human"), Some(1)),
            PlayerSlot::occupied(1, PlayerType::Human, Some("human"), Some(1)),
            PlayerSlot::occupied(2, PlayerType::Human, Some("orc"), Some(2)),
        ],
        "dots",
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        FinishPolicy::Endless,
        ruleset::demo(),
    );

    let own = Color::srgb(0.4, 0.85, 0.4);
    let allied = Color::srgb(0.95, 0.85, 0.35);
    let enemy = Color::srgb(1.0, 0.35, 0.35);
    let unowned = Color::srgb(0.6, 0.6, 0.65);
    assert_eq!(render::passenger_dot_color(&session, Some(0), Some(0)), own);
    assert_eq!(
        render::passenger_dot_color(&session, Some(0), Some(1)),
        allied
    );
    assert_eq!(
        render::passenger_dot_color(&session, Some(0), Some(2)),
        enemy
    );
    assert_eq!(
        render::passenger_dot_color(&session, Some(0), None),
        unowned
    );
    // With no viewed player, each dot is its owner's color: player 2's green.
    assert_eq!(
        render::passenger_dot_color(&session, None, Some(2)),
        Color::srgb(0.4, 0.8, 0.4)
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
    let (worker, _) = utils::create_entity(
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

/// Sets `entity`'s look, as a tick of the simulation would.
fn face(app: &mut App, entity: Entity, facing: Facing) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<LocationComponent>()
        .unwrap()
        .facing = facing;
}

/// Takes the snapshot the interpolation runs from, as the tick boundary does.
fn record_tick(app: &mut App) {
    app.world_mut()
        .run_system_once(render::record_prev)
        .expect("the tick is snapshotted");
}

/// The look the sprite is drawn along.
fn drawn_of(app: &App, entity: Entity) -> Facing {
    app.world()
        .get::<DrawnFacing>(entity)
        .expect("a drawn entity carries the look it is drawn at")
        .bearing()
}

/// Leaves the fixed clock half way to its next tick, where the drawing is half
/// way between the two.
fn part_way_through_tick(app: &mut App) {
    accumulate(app, 2)
}

/// Leaves it a whole tick on, which is as far as the drawing may go.
fn whole_tick(app: &mut App) {
    accumulate(app, 1)
}

/// Runs the fixed clock on by `1 / share` of a tick without advancing the
/// simulation, which is what a drawn frame between two ticks sees.
fn accumulate(app: &mut App, share: u32) {
    let mut fixed = app.world_mut().resource_mut::<Time<Fixed>>();
    fixed.set_timestep_hz(NOMINAL_TICK_HZ);
    let part = fixed.timestep() / share;
    fixed.accumulate_overstep(part);
}

/// The rotation the sprite is drawn at.
fn rotation_of(app: &App, entity: Entity) -> Quat {
    app.world().get::<Transform>(entity).unwrap().rotation
}

/// Draws one frame.
fn draw(app: &mut App) {
    app.world_mut()
        .run_system_once(render::interpolate_sprites)
        .expect("the sprites are drawn");
}

/// Which way the drawn shape's nose points, a shape pointing `+Y` at rest.
fn nose_of(rotation: Quat) -> Vec2 {
    (rotation * Vec3::Y).truncate()
}

/// Spawns `type_name` at a cell for the local player, with its sprite attached.
fn spawn_at(app: &mut App, type_name: &str, x: u32, y: u32) -> Entity {
    spawn_owned_at(app, type_name, x, y, 0)
}

/// Spawns `type_name` at a cell for `owner`, with its sprite attached.
fn spawn_owned_at(app: &mut App, type_name: &str, x: u32, y: u32, owner: PlayerId) -> Entity {
    let (entity, _) = utils::create_entity(
        app.world_mut(),
        type_name,
        utils::at_cell(x, y),
        Some(owner),
    )
    .unwrap_or_else(|| panic!("{type_name} spawns"));
    app.world_mut()
        .run_system_once(render::attach_sprites)
        .expect("sprites attach");
    app.world_mut()
        .entity_mut(entity)
        .insert(Visibility::default());
    entity
}

/// A mover carrying a gun that bears on its own — the combination where a single
/// drawn look would show the wrong thing. Its own registration rather than the
/// demo's war wagon, so a one-cell body with a centred mount keeps the geometry
/// here trivial whatever the demo rebalances.
fn register_gun_wagon(app: &mut App) {
    let ground = {
        let registry = app.world().resource::<ContentRegistry>();
        registry.layer("ground").expect("the demo declares ground")
    };
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    let gun = registry.register_turret(
        "wagon_gun",
        TurretDef::new(
            Weapon::new(ground, Delivery::Instant, None, Slain::Remains),
            TurretStats::default(),
            WeaponConduct::Halts,
        ),
    );
    registry.register(
        EntityTypeDef::new("gun_wagon")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_movement(
                FixedU64::from_num(0.3),
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(30),
                FixedU64::from_num(30),
            )
            .with_pool(Pool::health(40))
            .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(12))
            .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
            .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(4))
            .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
            .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
            .with_turrets([TurretMount::new(gun, CellPos::new(0, 0), CellSize::ONE)]),
    );
}

/// Trains `entity`'s gun on a bearing, as a tick of aiming would.
fn train(app: &mut App, entity: Entity, bearing: Facing) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<TurretsComponent>()
        .expect("a gun of its own")
        .0[0]
        .bearing = bearing;
}

/// The bearing half way from `from` to `to`, the short way round.
fn half_way(from: Facing, to: Facing) -> Facing {
    Facing::from_bits(
        from.to_bits()
            .wrapping_add((from.difference(to) / 2) as i16 as u16),
    )
}

/// The view-change clear, run once so it holds the view as it stands, with
/// the marker stores it clears.
fn view_change_clear(app: &mut App) -> impl System<In = (), Out = ()> {
    app.world_mut().init_resource::<Puffs>();
    app.world_mut().init_resource::<SkillPulses>();
    let mut forget = IntoSystem::into_system(render::forget_marks_on_view_change);
    forget.initialize(app.world_mut());
    forget
        .run((), app.world_mut())
        .expect("the view-change clear runs");
    forget
}

/// One puff and one pulse for player 0's view: a worker of player 0's made
/// out, then hidden, and a skill it casts on itself.
fn seed_marks(app: &mut App) {
    let worker = spawn_worker(app, "20");
    let worker_id = app
        .world()
        .get::<EntityInfoComponent>(worker)
        .expect("a spawned worker carries its id")
        .id();
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");
    let skill = app
        .world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("the demo content defines battle focus");
    app.world_mut()
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::SkillCast {
            caster: worker_id,
            target: SkillTarget::Entity(worker_id),
            skill,
        });
    app.world_mut()
        .run_system_once(render::collect_skill_pulses)
        .expect("the pulses are collected");
    spawn::hide_entity(app.world_mut(), worker, Inside::Nothing);
    app.world_mut()
        .run_system_once(render::collect_puffs)
        .expect("the markers are collected");
    assert_eq!(app.world().resource::<Puffs>().marked().len(), 1);
    assert_eq!(app.world().resource::<SkillPulses>().pulsed().len(), 1);
}
