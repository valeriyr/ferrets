//! Continuous-model movement around static footprints: a body pressed into a
//! building corner must round it and finish its walk, not converge on the
//! corner's tangent line forever.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    components::{location::LocationComponent, order_queue::OrderQueueComponent},
    map::Map,
    order::Order,
};

//
// ─── Corner rounding ────────────────────────────────────────────────────────
//

#[test]
fn body_pressed_into_footprint_corner_rounds_it() {
    let mut app = utils::corner_app();
    // The keep occupies cells (8..=10, 6..=8); the walk goes west along row 5,
    // tangent to the keep's north face.
    utils::spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 12, 5, 0);

    // A push carried the body off-lattice so its circle clips the keep's
    // north-east corner cell (10, 6): the westward axis is blocked, and the
    // free axis converges on the corner's tangent line without reaching it.
    app.world_mut()
        .entity_mut(runner)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = FixedUVec2::new(FixedU64::from_num(10.68), FixedU64::from_num(5.08));

    app.world_mut()
        .entity_mut(runner)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(4), FixedU64::from_num(5)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 300);

    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must finish instead of grinding at the corner"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), runner),
        CellPos::new(4, 5),
        "the runner rounded the corner and reached the ordered spot"
    );
}

//
// ─── Walled point orders ──────────────────────────────────────────────────────
//

#[test]
fn point_order_blocked_at_final_approach_accepts_by_cell() {
    let mut app = utils::corner_app();
    // The keep occupies cells (8..=10, 6..=8); the ordered spot lies on the
    // runner's own cell, but standing on it exactly would poke the body into
    // the keep's west face — the exact spot is unreachable forever.
    utils::spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 7, 7, 0);

    app.world_mut()
        .entity_mut(runner)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(7.3), FixedU64::from_num(7)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 60);

    // The frustration ring accepts by cells: each walled attempt escalates
    // and the escalation must stand through the regain walk — forgiving it
    // there laundered the frustration every round-trip and ground forever.
    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must finish instead of grinding at the wall"
    );
    assert_eq!(utils::cell_of(app.world_mut(), runner), CellPos::new(7, 7));
}

#[test]
fn body_clipped_into_footprint_walks_itself_free() {
    let mut app = utils::corner_app();
    utils::spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 7, 7, 0);

    // A building raised against the body's edge leaves the circle clipping
    // the static footprint — staged directly, since placement reads claims at
    // the rounded anchor and cannot see circle edges. Deep enough that no
    // single step clears it, so draining is the only way out.
    app.world_mut()
        .entity_mut(runner)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = FixedUVec2::new(FixedU64::from_num(7.45), FixedU64::from_num(7));

    app.world_mut()
        .entity_mut(runner)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(4), FixedU64::from_num(7)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 300);

    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the clipped body must walk itself free and finish"
    );
    assert_eq!(utils::cell_of(app.world_mut(), runner), CellPos::new(4, 7));
}

//
// ─── Yielding to a walk ────────────────────────────────────────────────────────
//

#[test]
fn wide_blocker_steps_aside_where_its_footprint_fits() {
    // A wall pins the lane from above and a stray block sits at (10, 6): the
    // step-aside must be measured with the blocker's own footprint and mask —
    // measured by single cells, the wagon is handed a spot its 2x2 body
    // cannot take (or backs into the walk's own face) and never clears the
    // way.
    let mut app = utils::cell_crowd_app();
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 6..=12 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
        }
        map.set_static_occupied(utils::GROUND, CellPos::new(10, 6), true);
    }
    let (wagon, _) = utils::spawn_owned(&mut app, "wagon", 8, 5, 0);
    let (soldier, _) = utils::spawn_owned(&mut app, "soldier", 6, 5, 0);

    app.world_mut()
        .entity_mut(soldier)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(12), FixedU64::from_num(5)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 200);

    assert_eq!(
        utils::cell_of(app.world_mut(), wagon),
        CellPos::new(8, 6),
        "the wagon stepped aside with its whole footprint"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(12, 5),
        "the walk passed through the vacated lane"
    );
}
