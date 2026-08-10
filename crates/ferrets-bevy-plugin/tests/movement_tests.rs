//! Continuous-model movement around static footprints: a body pressed into a
//! building corner must round it and finish its walk, not converge on the
//! corner's tangent line forever.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    components::{location::LocationComponent, order_queue::OrderQueueComponent},
    order::Order,
};
use utils::{cell_of, corner_app, run_ticks, spawn_owned};

//
// ─── Corner rounding ────────────────────────────────────────────────────────
//

#[test]
fn body_pressed_into_footprint_corner_rounds_it() {
    let mut app = corner_app();
    // The keep occupies cells (8..=10, 6..=8); the walk goes west along row 5,
    // tangent to the keep's north face.
    spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = spawn_owned(&mut app, "runner", 12, 5, 0);

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

    run_ticks(&mut app, 300);

    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must finish instead of grinding at the corner"
    );
    assert_eq!(
        cell_of(app.world_mut(), runner),
        CellPos::new(4, 5),
        "the runner rounded the corner and reached the ordered spot"
    );
}
