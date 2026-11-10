//! The claim plane: every claim on it is held by a body standing on the grid or a change's reservation.

mod utils;

use bevy::prelude::App;
use ferrets_geometry::{cell_pos::CellPos, projection::Projection};
use ferrets_simulation::{
    command::PlayerCommand, components::location::LocationComponent, entity_def, map::Map,
    movement_model::MovementModel, occupancy::GridMark, simulation_id::SimulationId, spawn,
};

//
// ─── The plane check ──────────────────────────────────────────────────────────
//

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(
    expected = "the claim plane holds exactly the claims of the bodies standing on the grid \
                and the reservations of changes under way"
)]
fn continuous_body_written_past_its_claim_fails_plane_check() {
    let mut app = utils::continuous_orders_app();
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    // Written around `move_body`: the claim stays on (2, 2).
    app.world_mut()
        .get_mut::<LocationComponent>(soldier)
        .expect("the soldier stands somewhere")
        .position = utils::pos(6, 2);
    utils::run_ticks(&mut app, 1);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(
    expected = "the claim plane holds exactly the claims of the bodies standing on the grid \
                and the reservations of changes under way"
)]
fn cell_body_written_past_its_claim_fails_plane_check() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Cell);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    // Written around `spawn::relocate`: the claim stays on (2, 2).
    app.world_mut()
        .get_mut::<LocationComponent>(soldier)
        .expect("the soldier stands somewhere")
        .position = utils::pos(6, 2);
    utils::run_ticks(&mut app, 1);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(
    expected = "the claim plane holds exactly the claims of the bodies standing on the grid \
                and the reservations of changes under way, each under its holder"
)]
fn claim_under_another_holder_fails_plane_check() {
    let mut app = utils::continuous_orders_app();
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    // The soldier's own cell, as many claims as before, but held under an id
    // no body has.
    let mark = entity_def::grid_mark(app.world(), soldier).expect("the soldier claims its cell");
    let mut map = app.world_mut().resource_mut::<Map>();
    map.unclaim(mark);
    map.claim(GridMark {
        holder: SimulationId(soldier_id.0 + 100),
        ..mark
    });
    utils::run_ticks(&mut app, 1);
}

//
// ─── Dying bodies ─────────────────────────────────────────────────────────────
//

#[test]
fn dying_body_keeps_its_claim_until_freed() {
    let mut app = utils::continuous_orders_app();
    let (_, survivor_id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);
    let (dying, dying_id) = utils::create_owned(&mut app, "soldier", 4, 2, 0);
    // Pressed onto the first soldier's cell — its middle at 2.9, its low edge
    // 2.4 anchoring to 2 — and killed there: a dying body pushes nothing, so
    // the survivor stays put.
    utils::move_body(&mut app, dying, utils::point("2.9", "2.5"));
    assert_eq!(holders_at(&app, 2, 2), [survivor_id.0, dying_id.0]);

    // The Die order starts on the first tick and runs the soldier's two
    // ticks of dying: its claim is held through tick 2 and freed on tick
    // 1 + 2 = 3, taking its own claim alone.
    spawn::destroy_entity(app.world_mut(), dying);
    utils::run_ticks(&mut app, 2);
    assert_eq!(
        holders_at(&app, 2, 2),
        [survivor_id.0, dying_id.0],
        "the dying soldier still holds its claim"
    );

    utils::run_ticks(&mut app, 1);
    assert_eq!(
        holders_at(&app, 2, 2),
        [survivor_id.0],
        "the survivor's claim outlasts the freed one"
    );
}

//
// ─── Moving a body with its claim ─────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a cell-model body moves crossing by crossing")]
fn moving_cell_model_body_with_its_claim_panics() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Cell);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    utils::move_body(&mut app, soldier, utils::pos(3, 2));
}

#[test]
#[should_panic(expected = "a body moved with its claim stands on the grid")]
fn moving_seated_body_with_its_claim_panics() {
    let mut app = utils::brood_app(MovementModel::Continuous);
    let (nurse, nurse_id) = utils::create_owned(&mut app, "nurse", 5, 5, 0);
    let (sow, sow_id) = utils::create_owned(&mut app, "sow", 6, 5, 0);
    // Mending the wounded sow, the nurse sits in its berths, off the grid
    // but still somewhere.
    utils::wound(&mut app, sow, "30");
    utils::select(&mut app, nurse_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: sow_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 15);
    assert!(entity_def::attached(app.world(), nurse));

    utils::move_body(&mut app, nurse, utils::pos(8, 5));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn holders_at(app: &App, x: u32, y: u32) -> Vec<u32> {
    app.world()
        .resource::<Map>()
        .nav_grid()
        .claim_holders(utils::GROUND, CellPos::new(x, y))
        .to_vec()
}
