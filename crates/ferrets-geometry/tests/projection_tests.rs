//! The projection's geometry: metric functions, step costs, point and
//! rectangle ranges, ranking distances, spans, and stepping kinematics.

mod utils;

use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection, Step},
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

//
// ─── chebyshev and octile ─────────────────────────────────────────────────────
//

#[test]
fn chebyshev_takes_maximum_of_axis_distances() {
    assert_eq!(projection::chebyshev(utils::nav(2, 3), utils::nav(2, 3)), 0);
    assert_eq!(projection::chebyshev(utils::nav(0, 0), utils::nav(3, 1)), 3);
    assert_eq!(projection::chebyshev(utils::nav(5, 2), utils::nav(1, 9)), 7);
}

#[test]
fn octile_charges_cardinal_runs_at_cardinal_cost() {
    assert_eq!(
        projection::octile(utils::nav(2, 5), utils::nav(6, 5)),
        4 * Projection::Orthogonal.step_cost(Step::Cardinal)
    );
}

#[test]
fn octile_charges_diagonal_runs_at_diagonal_cost() {
    assert_eq!(
        projection::octile(utils::nav(1, 1), utils::nav(4, 4)),
        3 * Projection::Orthogonal.step_cost(Step::Diagonal)
    );
}

#[test]
fn octile_mixes_diagonal_and_cardinal_legs() {
    // (0,0) → (3,1): one diagonal step covers the y delta, two cardinal
    // steps cover the rest of x.
    assert_eq!(
        projection::octile(utils::nav(0, 0), utils::nav(3, 1)),
        Projection::Orthogonal.step_cost(Step::Diagonal)
            + 2 * Projection::Orthogonal.step_cost(Step::Cardinal)
    );
}

//
// ─── step_cost ────────────────────────────────────────────────────────────────
//

#[test]
fn isometric_steps_cost_same_in_all_directions() {
    assert_eq!(Projection::Isometric.step_cost(Step::Cardinal), 10);
    assert_eq!(Projection::Isometric.step_cost(Step::Diagonal), 10);
}

#[test]
fn orthogonal_diagonal_steps_cost_more() {
    // 14 approximates √2 × the cardinal 10 in integer costs.
    assert_eq!(Projection::Orthogonal.step_cost(Step::Cardinal), 10);
    assert_eq!(Projection::Orthogonal.step_cost(Step::Diagonal), 14);
}

//
// ─── metric ───────────────────────────────────────────────────────────────────
//

#[test]
fn isometric_metric_is_chebyshev_at_cardinal_cost() {
    let (a, b) = (utils::nav(1, 2), utils::nav(5, 4));

    assert_eq!(
        Projection::Isometric.metric(a, b),
        projection::chebyshev(a, b) * Projection::Isometric.step_cost(Step::Cardinal)
    );
}

#[test]
fn orthogonal_metric_is_octile() {
    let (a, b) = (utils::nav(1, 2), utils::nav(5, 4));

    assert_eq!(
        Projection::Orthogonal.metric(a, b),
        projection::octile(a, b)
    );
}

//
// ─── in_range ─────────────────────────────────────────────────────────────────
//
// Distance between two single cells:
//
// . . . . . . .   y=0
// . . . . . . .   y=1
// . . A . . . .   y=2   A = (2,2), B = (5,5)
// . . . . . . .   y=3   Isometric(Chebyshev):  max(3,3) = 3
// . . . . . . .   y=4   Orthogonal(Euclidean): √(3²+3²) ≈ 4.24
// . . . . . B .   y=5
// . . . . . . .   y=6
//

#[test]
fn in_range_zero_distance_requires_exact_position() {
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(projection.in_range(utils::nav(3, 3), utils::nav(3, 3), 0));
        assert!(!projection.in_range(utils::nav(3, 3), utils::nav(4, 3), 0));
    }
}

#[test]
fn in_range_cardinal_distance_matches_in_both_projections() {
    // (3,0) → (5,0): 2 cells apart on one axis — both metrics agree.
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(projection.in_range(utils::nav(3, 0), utils::nav(5, 0), 2));
        assert!(!projection.in_range(utils::nav(3, 0), utils::nav(5, 0), 1));
    }
}

#[test]
fn in_range_diagonal_is_chebyshev_for_isometric() {
    // (2,2) → (5,5): Chebyshev max(3,3) = 3.
    assert!(Projection::Isometric.in_range(utils::nav(2, 2), utils::nav(5, 5), 3));
    assert!(!Projection::Isometric.in_range(utils::nav(2, 2), utils::nav(5, 5), 2));
}

#[test]
fn in_range_diagonal_is_euclidean_for_orthogonal() {
    // (2,2) → (5,5): Euclidean √18 ≈ 4.24 — more than 4, within 5.
    assert!(!Projection::Orthogonal.in_range(utils::nav(2, 2), utils::nav(5, 5), 4));
    assert!(Projection::Orthogonal.in_range(utils::nav(2, 2), utils::nav(5, 5), 5));
}

#[test]
fn in_range_pythagorean_triple_is_exact_for_orthogonal() {
    // (0,3) → (4,0): a 3-4-5 triangle — Euclidean distance exactly 5; Chebyshev 4.
    assert!(Projection::Orthogonal.in_range(utils::nav(0, 3), utils::nav(4, 0), 5));
    assert!(!Projection::Orthogonal.in_range(utils::nav(0, 3), utils::nav(4, 0), 4));
    assert!(Projection::Isometric.in_range(utils::nav(0, 3), utils::nav(4, 0), 4));
}

#[test]
fn in_range_survives_map_scale_distances() {
    // 60000 cells per axis square in u32 — the check widens internally, so
    // huge maps rank correctly instead of overflowing: the true diagonal is
    // 60000·√2 ≈ 84852.8.
    let (a, b) = (utils::nav(0, 0), utils::nav(60000, 60000));

    assert!(!Projection::Orthogonal.in_range(a, b, 84852));
    assert!(Projection::Orthogonal.in_range(a, b, 84853));
    assert!(Projection::Isometric.in_range(a, b, 60000));
}

//
// ─── in_range_of_rect ─────────────────────────────────────────────────────────
//
// All tests use a 2×2 rectangle at origin (3,3):
//
// . . . . . . .   y=0
// . . . . . . .   y=1
// . . . . . . .   y=2
// . . . R R . .   y=3   R = rectangle cells (3,3) (4,3)
// . . . R R . .   y=4                       (3,4) (4,4)
// . . . . . . .   y=5
// . . . . . . .   y=6
//

#[test]
fn rect_position_inside_is_at_distance_zero() {
    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        assert!(in_range_of_rect_iso(cell, 0));
        assert!(in_range_of_rect_ortho(cell, 0));
    }
}

#[test]
fn rect_cardinally_adjacent_position_is_at_distance_one() {
    // Touching each side: left of (3,3), right of (4,4), above (4,3), below (3,4).
    for cell in [(2, 3), (5, 4), (4, 2), (3, 5)] {
        assert!(!in_range_of_rect_iso(cell, 0));
        assert!(in_range_of_rect_iso(cell, 1));
        assert!(!in_range_of_rect_ortho(cell, 0));
        assert!(in_range_of_rect_ortho(cell, 1));
    }
}

#[test]
fn rect_distance_is_measured_to_nearest_cell_not_origin() {
    // (6,4) is 3 from the origin (3,3) but only 2 from the nearest cell (4,4).
    assert!(in_range_of_rect_iso((6, 4), 2));
    assert!(in_range_of_rect_ortho((6, 4), 2));
    assert!(!in_range_of_rect_iso((6, 4), 1));
    assert!(!in_range_of_rect_ortho((6, 4), 1));
}

#[test]
fn rect_isometric_treats_diagonal_corner_as_distance_one() {
    // (2,2) touches the rect corner (3,3) diagonally — Chebyshev 1.
    assert!(in_range_of_rect_iso((2, 2), 1));
}

#[test]
fn rect_orthogonal_treats_diagonal_corner_as_farther_than_one() {
    // (2,2) → nearest cell (3,3): Euclidean √2 > 1, but ≤ 2.
    assert!(!in_range_of_rect_ortho((2, 2), 1));
    assert!(in_range_of_rect_ortho((2, 2), 2));
}

#[test]
fn rect_corner_clamps_on_both_axes() {
    // (6,6) → nearest cell (4,4): Chebyshev 2; Euclidean √8 ≈ 2.83.
    assert!(in_range_of_rect_iso((6, 6), 2));
    assert!(!in_range_of_rect_ortho((6, 6), 2));
    assert!(in_range_of_rect_ortho((6, 6), 3));
}

#[test]
fn rect_single_cell_matches_in_range() {
    // A 1×1 rectangle degenerates to a plain point-range check.
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        for from in [(3, 3), (1, 3), (0, 0), (6, 2)] {
            for distance in 0..4 {
                assert_eq!(
                    projection.in_range_of_rect(
                        utils::nav(from.0, from.1),
                        CellRect::cell(RECT_ORIGIN),
                        distance
                    ),
                    projection.in_range(utils::nav(from.0, from.1), RECT_ORIGIN, distance),
                    "projection {projection:?}, from {from:?}, distance {distance}"
                );
            }
        }
    }
}

#[test]
fn rect_wide_clamps_to_facing_side() {
    // 3×1 rectangle: cells (3,3) (4,3) (5,3).
    // (4,5) is below the middle cell (4,3): distance 2 on both metrics.
    let rect = CellRect::new(RECT_ORIGIN, CellSize::new(3, 1));
    let from = utils::nav(4, 5);

    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert!(projection.in_range_of_rect(from, rect, 2));
        assert!(!projection.in_range_of_rect(from, rect, 1));
    }
}

//
// ─── rect_distance ────────────────────────────────────────────────────────────
//
// The first two tests use the same 2×2 rectangle at origin (3,3), with P the
// probe point (7,4) whose nearest rectangle cell is (4,4):
//
// . . . . . . . . .   y=0
// . . . . . . . . .   y=1
// . . . . . . . . .   y=2
// . . . R R . . . .   y=3   R = rectangle cells (3,3) (4,3)
// . . . R R . . P .   y=4                       (3,4) (4,4)
// . . . . . . . . .   y=5
// . . . . . . . . .   y=6
//

#[test]
fn rect_distance_is_zero_inside_footprint() {
    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        assert_eq!(rect_distance_iso(cell), 0);
        assert_eq!(rect_distance_ortho(cell), 0);
    }
}

#[test]
fn rect_distance_measures_to_nearest_cell() {
    // (7,4) → nearest cell (4,4): 3 cells away along the x axis.
    assert_eq!(rect_distance_iso((7, 4)), 3); // Chebyshev cells
    assert_eq!(rect_distance_ortho((7, 4)), 9); // squared Euclidean cells
}

// Against a single-cell rect R at the origin, two candidate cells A and B:
//
// R . . . . . .   y=0   R = rectangle cell (0,0)
// . . . . . . .   y=1   A = diagonal candidate (5,5)
// . . . . . . .   y=2   B = cardinal candidate (0,6)
// . . . . . . .   y=3
// . . . . . . .   y=4
// . . . . . A .   y=5   A: Chebyshev 5, Euclidean² 50
// B . . . . . .   y=6   B: Chebyshev 6, Euclidean² 36
//
// Isometric ranks the diagonal candidate A closer; Orthogonal, which charges
// diagonals more ground, ranks the cardinal candidate B closer.
#[test]
fn rect_distance_ranks_diagonal_vs_cardinal_by_projection() {
    let a = utils::nav(5, 5);
    let b = utils::nav(0, 6);
    let origin = CellRect::cell(utils::nav(0, 0));

    assert!(
        Projection::Isometric.rect_distance(a, origin)
            < Projection::Isometric.rect_distance(b, origin)
    );
    assert!(
        Projection::Orthogonal.rect_distance(a, origin)
            > Projection::Orthogonal.rect_distance(b, origin)
    );
}

//
// ─── ring_floor ───────────────────────────────────────────────────────────────
//

/// Under the Chebyshev metric every cell of scan ring `radius` ranks exactly
/// `radius`, so the floor is the radius itself.
#[test]
fn isometric_ring_floor_is_radius() {
    assert_eq!(Projection::Isometric.ring_floor(0), 0);
    assert_eq!(Projection::Isometric.ring_floor(3), 3);
}

/// Under the squared-Euclidean rank the cheapest cell of scan ring `radius`
/// is a cardinal one at `radius²`; diagonal cells of the ring rank higher.
#[test]
fn orthogonal_ring_floor_is_squared_radius() {
    assert_eq!(Projection::Orthogonal.ring_floor(0), 0);
    assert_eq!(Projection::Orthogonal.ring_floor(3), 9);
}

/// The floor never exceeds the rank of any actual ring cell: the cardinal
/// cell sits exactly on it, the diagonal one above it.
#[test]
fn ring_floor_bounds_actual_ring_ranks() {
    let goal = CellRect::new(utils::nav(10, 10), CellSize::new(1, 1));
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        let cardinal = projection.rect_distance(utils::nav(13, 10), goal);
        let diagonal = projection.rect_distance(utils::nav(13, 13), goal);
        assert_eq!(projection.ring_floor(3), cardinal);
        assert!(projection.ring_floor(3) <= diagonal);
    }
}

//
// ─── span ─────────────────────────────────────────────────────────────────────
//

#[test]
fn span_takes_dominant_axis_under_isometric() {
    assert_eq!(
        Projection::Isometric.span(FixedU64::from_num(3), FixedU64::from_num(4)),
        FixedU64::from_num(4)
    );
}

#[test]
fn span_takes_euclidean_length_under_orthogonal() {
    assert_eq!(
        Projection::Orthogonal.span(FixedU64::from_num(3), FixedU64::from_num(4)),
        FixedU64::from_num(5)
    );
}

#[test]
fn span_of_single_axis_offset_is_its_length() {
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert_eq!(
            projection.span(FixedU64::from_num(2.5), FixedU64::ZERO),
            FixedU64::from_num(2.5)
        );
        assert_eq!(
            projection.span(FixedU64::ZERO, FixedU64::ZERO),
            FixedU64::ZERO
        );
    }
}

//
// ─── step_toward ──────────────────────────────────────────────────────────────
//

#[test]
fn isometric_step_advances_both_axes_at_full_rate() {
    let stepped =
        Projection::Isometric.step_toward(utils::world(0, 0), utils::world(10, 10), FixedU64::ONE);

    assert_eq!(stepped, utils::world(1, 1));
}

#[test]
fn isometric_step_scales_minor_axis_proportionally() {
    // Toward (3,1) at speed 1: the dominant axis advances at full rate and
    // the minor one at a third — one cell of Chebyshev ground per tick, so
    // a sideways-deflected mover eases back instead of snapping.
    let stepped =
        Projection::Isometric.step_toward(utils::world(0, 0), utils::world(3, 1), FixedU64::ONE);

    assert_eq!(stepped.x, FixedU64::ONE);
    assert_eq!(stepped.y, FixedU64::ONE / FixedU64::from_num(3));
}

#[test]
fn orthogonal_step_normalizes_diagonal_travel() {
    let stepped =
        Projection::Orthogonal.step_toward(utils::world(0, 0), utils::world(10, 10), FixedU64::ONE);

    // Each axis advances by 1/√2, so one cell of diagonal ground costs √2 time.
    assert!(stepped.x > FixedU64::from_num(0.7069) && stepped.x < FixedU64::from_num(0.7072));
    assert_eq!(stepped.x, stepped.y);
}

#[test]
fn orthogonal_cardinal_step_moves_at_full_speed() {
    let stepped =
        Projection::Orthogonal.step_toward(utils::world(0, 5), utils::world(10, 5), FixedU64::ONE);

    assert_eq!(stepped, utils::world(1, 5));
}

#[test]
fn step_within_reach_lands_exactly_on_target() {
    let target = FixedUVec2::new(FixedU64::from_num(0.5), FixedU64::from_num(0.5));

    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert_eq!(
            projection.step_toward(utils::world(0, 0), target, FixedU64::ONE),
            target,
            "arrival must snap exactly so waypoints pop"
        );
    }
}

#[test]
fn step_at_target_stays_at_target() {
    let target = utils::world(4, 4);

    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert_eq!(
            projection.step_toward(target, target, FixedU64::ONE),
            target
        );
    }
}

#[test]
fn step_with_zero_speed_stands_still() {
    // A rooted mover: zero speed means zero progress, on any heading.
    for projection in [Projection::Isometric, Projection::Orthogonal] {
        assert_eq!(
            projection.step_toward(utils::world(2, 2), utils::world(7, 5), FixedU64::ZERO),
            utils::world(2, 2)
        );
    }
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Origin of the rectangle used by the `in_range_of_rect` tests.
const RECT_ORIGIN: CellPos = CellPos::new(3, 3);
/// Footprint of the rectangle used by the `in_range_of_rect` tests.
const RECT_SIZE: CellSize = CellSize::new(2, 2);

fn in_range_of_rect_iso(from: (u32, u32), distance: u32) -> bool {
    Projection::Isometric.in_range_of_rect(
        utils::nav(from.0, from.1),
        CellRect::new(RECT_ORIGIN, RECT_SIZE),
        distance,
    )
}

fn in_range_of_rect_ortho(from: (u32, u32), distance: u32) -> bool {
    Projection::Orthogonal.in_range_of_rect(
        utils::nav(from.0, from.1),
        CellRect::new(RECT_ORIGIN, RECT_SIZE),
        distance,
    )
}

fn rect_distance_iso(from: (u32, u32)) -> u32 {
    Projection::Isometric.rect_distance(
        utils::nav(from.0, from.1),
        CellRect::new(RECT_ORIGIN, RECT_SIZE),
    )
}

fn rect_distance_ortho(from: (u32, u32)) -> u32 {
    Projection::Orthogonal.rect_distance(
        utils::nav(from.0, from.1),
        CellRect::new(RECT_ORIGIN, RECT_SIZE),
    )
}
