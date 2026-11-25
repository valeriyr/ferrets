//! Tests for a continuous walk's progress record and pursued waypoint.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_simulation::components::movement::{Landing, Measured, MoveComponent, Pop, Pursued};

//
// ─── Baseline ─────────────────────────────────────────────────────────────────
//

/// A regained position forgives nothing, nor does the first distance after
/// it: a walk escalated twice, regaining its anchor with its detour unspent,
/// pops it and measures the next waypoint — a baseline that leaves both
/// escalations standing.
#[test]
fn first_distance_after_regained_pop_forgives_nothing() {
    let mut walk = escalated_twice();
    walk.regain(CellPos::new(4, 4));

    walk.pop_waypoint(Pop::Reached(point("4.5", "4.5")));
    let measured = walk.measure(utils::fixed("3"));

    assert_eq!(
        (measured, walk.frustration, walk.pursued),
        (Measured::Baseline, 2, Pursued::Ordinary)
    );
}

/// The first distance after a shove that repaths the walk forgives nothing:
/// a walk escalated twice, shoved and told to repath around claims, measures
/// its target afresh — a baseline that leaves both escalations standing.
#[test]
fn first_distance_after_repath_forgives_nothing() {
    let mut walk = escalated_twice();
    walk.measure(utils::fixed("3"));

    walk.repath_avoiding_claims();
    let measured = walk.measure(utils::fixed("2"));

    // 2 < 3, but the record went with the plan: a baseline, not a record.
    assert_eq!((measured, walk.frustration), (Measured::Baseline, 2));
}

/// A handed-back pop forgives nothing, and its next waypoint starts with a
/// baseline: a walk escalated twice pops a waypoint, is handed back, and
/// measures the next one.
#[test]
fn handed_back_pop_forgives_nothing() {
    let mut walk = escalated_twice();

    walk.pop_waypoint(Pop::Reached(point("5.5", "5.5")));
    walk.land(Landing::HandedBack);
    let measured = walk.measure(utils::fixed("1"));

    assert_eq!((measured, walk.frustration), (Measured::Baseline, 2));
}

//
// ─── Replaced plans ───────────────────────────────────────────────────────────
//

/// A plan replaced outright leaves no position being regained: a walk
/// regaining its anchor that takes a new route, or is told to repath around
/// claims, pursues ordinary waypoints.
#[test]
fn replaced_plan_leaves_regaining() {
    let mut routed = escalated_twice();
    routed.regain(CellPos::new(4, 4));
    routed.take_route(vec![CellPos::new(6, 5)]);

    let mut repathed = escalated_twice();
    repathed.regain(CellPos::new(4, 4));
    repathed.repath_avoiding_claims();

    assert_eq!(
        (routed.pursued, repathed.pursued),
        (Pursued::Ordinary, Pursued::Ordinary)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A walk on (4, 4) pursuing (5, 5) then (6, 5), escalated twice.
fn escalated_twice() -> MoveComponent {
    let mut walk = MoveComponent::new(CellPos::new(4, 4));
    walk.pursue_segment(vec![CellPos::new(5, 5), CellPos::new(6, 5)]);
    walk.escalate();
    walk.escalate();
    walk
}

/// The point at `x`, `y`.
fn point(x: &str, y: &str) -> FixedUVec2 {
    FixedUVec2::new(utils::fixed(x), utils::fixed(y))
}
