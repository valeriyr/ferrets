//! Selection: combine modes, box priority, and select-by-type.

mod utils;

use ferrets_math::fixed_urect::FixedURect;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    spawn,
};

//
// ─── Combine modes ───────────────────────────────────────────────────────────
//

#[test]
fn shift_add_extends_selection() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();

    utils::select(&mut app, a);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: b,
            mode: SelectMode::Add,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a, b]);
}

#[test]
fn toggle_removes_selected_and_adds_unselected() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();

    utils::select(&mut app, a);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: a,
            mode: SelectMode::Toggle,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: b,
            mode: SelectMode::Toggle,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![b]);
}

#[test]
fn remove_mode_subtracts_from_selection() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: rect((4, 4), (8, 8)),
            mode: SelectMode::Replace,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: b,
            mode: SelectMode::Remove,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a]);
}

//
// ─── Box priority ────────────────────────────────────────────────────────────
//

#[test]
fn box_select_excludes_buildings() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, unit) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    spawn::create_entity(world, "keep", utils::pos(7, 7), Some(0)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: rect((4, 4), (10, 10)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![unit]);
}

#[test]
fn box_select_keeps_only_own_units_when_present() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, own) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: rect((4, 4), (8, 8)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![own]);
}

#[test]
fn box_select_falls_back_to_single_other_owner() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    // A scout outside the box keeps the enemies in sight — a boxed enemy is
    // selectable for inspection only while someone actually sees it.
    spawn::create_entity(world, "soldier", utils::pos(3, 5), Some(0)).unwrap();
    let (_, first) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(1)).unwrap();
    spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: rect((4, 4), (8, 8)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // No own units in the box, so a single enemy is kept for inspection.
    assert_eq!(utils::selection(&app), vec![first]);
}

//
// ─── Select by type ──────────────────────────────────────────────────────────
//

#[test]
fn select_by_type_grabs_own_class_on_screen() {
    let mut app = utils::selection_app();
    let world = app.world_mut();
    let (_, a) = spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, b) = spawn::create_entity(world, "soldier", utils::pos(6, 5), Some(0)).unwrap();
    // A same-class enemy and a different-class own unit are both excluded.
    spawn::create_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();
    spawn::create_entity(world, "critter", utils::pos(8, 5), Some(0)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByType {
            class: "soldier".into(),
            rect: rect((0, 0), (20, 20)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![a, b]);
}

//
// ─── Fog of war ──────────────────────────────────────────────────────────────
//

#[test]
fn fogged_enemy_is_not_selectable_by_id() {
    // The enemy stands far beyond every own unit's sight: naming it selects
    // nothing — the fog that hides the sprite hides the stats too.
    let mut app = utils::selection_app();
    let world = app.world_mut();
    spawn::create_entity(world, "soldier", utils::pos(3, 5), Some(0)).unwrap();
    let (_, fogged) = spawn::create_entity(world, "soldier", utils::pos(25, 25), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: fogged,
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![]);
}

#[test]
fn boxed_fogged_enemy_is_not_kept_for_inspection() {
    // The same box as the fallback test, with nobody watching it: the enemy
    // inside is fogged, so the box selects nothing at all.
    let mut app = utils::selection_app();
    let world = app.world_mut();
    spawn::create_entity(world, "soldier", utils::pos(25, 25), Some(0)).unwrap();
    spawn::create_entity(world, "soldier", utils::pos(5, 5), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: rect((4, 4), (8, 8)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::selection(&app), vec![]);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// A rectangle spanning the given inclusive cell corners.
fn rect(min: (u32, u32), max: (u32, u32)) -> FixedURect {
    FixedURect::from_corners(utils::pos(min.0, min.1), utils::pos(max.0, max.1))
}
