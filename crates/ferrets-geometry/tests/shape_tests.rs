//! Shapes on the map: the cells they lie over, their gaps and nearest points.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, shape::Shape};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};

//
// ─── Cells of a box ───────────────────────────────────────────────────────────
//

#[test]
fn one_cell_box_is_that_cell() {
    let cell = Shape::cell(utils::nav(5, 5));

    assert_eq!(cell.cells(), vec![utils::nav(5, 5)]);
    // 5 + 1/2 on each axis.
    assert_eq!(cell.middle(), utils::point("5.5", "5.5"));
}

#[test]
fn lined_up_box_covers_its_own_cells() {
    // A 2×1 on (5, 5): bounds 5–7 by 5–6.
    let shape = boxed(("5", "5"), ("7", "6"));

    assert_eq!(
        shape.cell_bounds(),
        CellRect::new(utils::nav(5, 5), CellSize::new(2, 1))
    );
    assert_eq!(shape.cells(), vec![utils::nav(5, 5), utils::nav(6, 5)]);
}

#[test]
fn box_short_of_half_cell_covers_cell_ahead() {
    // 9.3 to 10.3 on x: cells 9 and 10; 4 to 5 on y: row 4.
    assert_eq!(
        boxed(("9.3", "4"), ("10.3", "5")).cell_bounds(),
        CellRect::new(utils::nav(9, 4), CellSize::new(2, 1))
    );
}

#[test]
fn box_past_corner_covers_four_cells() {
    // 9.3 to 10.3 on x, 4.8 to 5.8 on y: cells 9–10 by 4–5.
    assert_eq!(
        boxed(("9.3", "4.8"), ("10.3", "5.8")).cell_bounds(),
        CellRect::new(utils::nav(9, 4), CellSize::new(2, 2))
    );
}

#[test]
fn wide_box_across_boundary_covers_full_span() {
    // A 2×2 from x 5.6 to 7.6: cells 5, 6 and 7.
    assert_eq!(
        boxed(("5.6", "5"), ("7.6", "7")).cell_bounds(),
        CellRect::new(utils::nav(5, 5), CellSize::new(3, 2))
    );
}

#[test]
fn box_middle_lies_halfway_across_its_bounds() {
    // (5.6 + 7.6) / 2 = 6.6, (5 + 7) / 2 = 6.
    assert_eq!(
        boxed(("5.6", "5"), ("7.6", "7")).middle(),
        utils::point("6.6", "6")
    );
}

#[test]
fn circle_has_its_radius_and_box_none() {
    assert_eq!(
        circle(("2.5", "2.5"), "0.75").radius(),
        Some(utils::fixed("0.75"))
    );
    assert_eq!(Shape::cell(CellPos::new(2, 2)).radius(), None);
}

//
// ─── Cells of a circle ────────────────────────────────────────────────────────
//

#[test]
fn circle_short_of_half_cell_spans_cell_ahead() {
    // A half-cell radius around x 9.8 runs from 9.3 to 10.3: cells 9 and 10 on
    // x, row 4 alone on y (4.0 to 5.0).
    assert_eq!(
        circle(("9.8", "4.5"), "0.5").cell_bounds(),
        CellRect::new(utils::nav(9, 4), CellSize::new(2, 1))
    );
}

#[test]
fn circle_bounds_span_corner_its_circle_misses() {
    // Around (9.6, 9.6) the circle runs from 9.1 to 10.1 on each axis, so its
    // box spans 9–10 by 9–10. The far corner (10, 10) lies
    // √(0.4² + 0.4²) ≈ 0.57 from the middle, past the 0.5 radius: the circle
    // overlaps three of the box's four cells.
    let shape = circle(("9.6", "9.6"), "0.5");

    assert_eq!(
        shape.cell_bounds(),
        CellRect::new(utils::nav(9, 9), CellSize::new(2, 2))
    );
    assert_eq!(
        shape.cells(),
        vec![utils::nav(9, 9), utils::nav(10, 9), utils::nav(9, 10)]
    );
}

#[test]
fn circle_of_no_radius_lies_over_cell_its_middle_lies_in() {
    // No radius around (6, 6), the corner four cells share: the one cell the
    // point lies in, (6, 6).
    let shape = circle(("6", "6"), "0");

    assert_eq!(shape.cell_bounds(), CellRect::cell(utils::nav(6, 6)));
    assert_eq!(shape.cells(), vec![utils::nav(6, 6)]);
}

/// A circle lined up on its cell touches its neighbors' boundaries exactly,
/// and touching does not enter:
///
///   ┌───┐
///   │(○)│   one cell, four boundary touches
///   └───┘
#[test]
fn lined_up_circle_overlaps_one_cell() {
    assert_eq!(
        circle(("5.5", "5.5"), "0.5").cells(),
        vec![utils::nav(5, 5)]
    );
}

#[test]
fn circle_straddling_border_overlaps_both_cells() {
    // Around x 5.8 the circle runs from 5.3 to 6.3.
    assert_eq!(
        circle(("5.8", "5.5"), "0.5").cells(),
        vec![utils::nav(5, 5), utils::nav(6, 5)]
    );
}

#[test]
fn circle_near_corner_overlaps_four_cells() {
    assert_eq!(
        circle(("5.8", "5.8"), "0.5").cells(),
        vec![
            utils::nav(5, 5),
            utils::nav(6, 5),
            utils::nav(5, 6),
            utils::nav(6, 6),
        ]
    );
}

#[test]
fn wide_circle_overlaps_its_block() {
    // A radius of 1 around (5.5, 5.5) spans 4.5 to 6.5: three cells a side,
    // and each corner cell's nearest point lies √(0.5² + 0.5²) ≈ 0.71 away.
    assert_eq!(circle(("5.5", "5.5"), "1").cells().len(), 9);
}

/// The inscribed circle of a two-cell footprint lined up on its cells reaches
/// exactly its own four cells and no further:
///
///   ┌───┬───┐
///   │ ( │ ) │   four cells, boundaries touched but not crossed
///   ├───┼───┤
///   │ ( │ ) │
///   └───┴───┘
#[test]
fn inscribed_circle_of_two_cell_footprint_overlaps_exactly_it() {
    // A 2×2 on (5, 5) has its middle at 5 + 2/2 = 6.
    assert_eq!(
        circle(("6", "6"), "1").cells(),
        vec![
            utils::nav(5, 5),
            utils::nav(6, 5),
            utils::nav(5, 6),
            utils::nav(6, 6),
        ]
    );
}

#[test]
fn inscribed_circle_never_leaves_its_footprint() {
    for size in [1, 2, 3] {
        // At rest on (5, 5): the middle at 5 + size/2, the radius size/2.
        let half = FixedU64::from_num(size) / 2;
        let middle = FixedU64::from_num(5) + half;
        let cells = Shape::Circle {
            middle: FixedUVec2::new(middle, middle),
            radius: half,
        }
        .cells();
        assert_eq!(
            cells,
            CellRect::new(utils::nav(5, 5), CellSize::new(size, size))
                .cells()
                .collect::<Vec<CellPos>>(),
            "a {size}-cell body's circle overlaps its own cells alone"
        );
    }
}

#[test]
fn circle_wider_than_any_footprint_overlaps_its_disc() {
    // A radius of 10 around (20.5, 20.5): every cell whose nearest point lies
    // under 10 away. Its box spans 10.5 to 30.5, cells 10 to 30, 21 a side;
    // the disc leaves out each corner's cells farther than 10.
    let disc = circle(("20.5", "20.5"), "10");
    let cells = disc.cells();

    assert_eq!(
        disc.cell_bounds(),
        CellRect::new(utils::nav(10, 10), CellSize::new(21, 21))
    );
    // A cell k rows off the middle's lies k − 0.5 away on y (0 on its own
    // row), and one j columns off j − 0.5 away on x (0 on its own column);
    // kept while (j − 0.5)² + (k − 0.5)² < 100. Rows k = 0…10 keep 21, 21,
    // 21, 21, 19, 19, 17, 17, 15, 11, 7 cells; the middle row once, every
    // other twice: 21 + 2 · 168 = 357.
    let row = |k: u32| cells.iter().filter(|cell| cell.y == 20 + k).count();
    assert_eq!(
        (0..=10).map(row).collect::<Vec<_>>(),
        vec![21, 21, 21, 21, 19, 19, 17, 17, 15, 11, 7]
    );
    assert_eq!(cells.len(), 357);
}

//
// ─── Distance ─────────────────────────────────────────────────────────────────
//

#[test]
fn boxes_touching_on_axis_are_no_distance_apart() {
    let (from, to) = (Shape::cell(utils::nav(5, 5)), Shape::cell(utils::nav(6, 5)));

    assert_eq!(from.distance(to), FixedU64::ZERO);
    assert!(from.within(to, FixedU64::ZERO));
}

#[test]
fn boxes_touching_at_corner_are_no_distance_apart() {
    let (from, to) = (Shape::cell(utils::nav(5, 5)), Shape::cell(utils::nav(6, 6)));

    assert_eq!(from.distance(to), FixedU64::ZERO);
    assert!(from.within(to, FixedU64::ZERO));
}

#[test]
fn overlapping_boxes_are_no_distance_apart() {
    let from = boxed(("5", "5"), ("7", "7"));
    let to = boxed(("6.5", "6.5"), ("8", "8"));

    assert_eq!(from.distance(to), FixedU64::ZERO);
}

#[test]
fn boxes_apart_on_axis_are_their_gap_apart() {
    // Cell 5 ends at 6, cell 8 starts at 8: 8 − 6 = 2.
    let (from, to) = (Shape::cell(utils::nav(5, 5)), Shape::cell(utils::nav(8, 5)));

    assert_eq!(from.distance(to), FixedU64::from_num(2));
    assert!(from.within(to, FixedU64::from_num(2)));
    assert!(!from.within(to, utils::fixed("1.9")));
}

#[test]
fn boxes_apart_diagonally_are_their_euclidean_gap_apart() {
    // Gaps of 8 − 6 = 2 on x and 9 − 6 = 3 on y: √(2² + 3²) = √13 ≈ 3.606,
    // rounded down to the last fractional bit.
    let (from, to) = (Shape::cell(utils::nav(5, 5)), Shape::cell(utils::nav(8, 9)));

    assert_eq!(
        from.distance(to),
        FixedU64::from_bits((13u128 << 64).isqrt() as u64)
    );
    assert!(from.within(to, utils::fixed("3.61")));
    assert!(!from.within(to, utils::fixed("3.6")));
}

#[test]
fn circle_and_box_are_gap_from_circle_edge_apart() {
    // The box's nearest point to (2.5, 5.5) is (5, 5.5), 2.5 away; less the
    // 0.5 radius, 2.
    let (from, to) = (circle(("2.5", "5.5"), "0.5"), Shape::cell(utils::nav(5, 5)));

    assert_eq!(from.distance(to), FixedU64::from_num(2));
    assert_eq!(to.distance(from), FixedU64::from_num(2));
}

#[test]
fn circles_apart_are_gap_between_edges_apart() {
    // Middles 3 and 4 apart: √(3² + 4²) = 5; less radii 1 and 1.5, 2.5.
    let (from, to) = (circle(("2", "2"), "1"), circle(("5", "6"), "1.5"));

    assert_eq!(from.distance(to), utils::fixed("2.5"));
    assert!(from.within(to, utils::fixed("2.5")));
    assert!(!from.within(to, FixedU64::from_bits(utils::fixed("2.5").to_bits() - 1)));
}

#[test]
fn touching_circles_are_no_distance_apart() {
    // Middles 5 apart, radii 2 and 3.
    let (from, to) = (circle(("2", "2"), "2"), circle(("5", "6"), "3"));

    assert_eq!(from.distance(to), FixedU64::ZERO);
    assert!(from.within(to, FixedU64::ZERO));
}

#[test]
fn overlapping_circles_are_no_distance_apart() {
    let (from, to) = (circle(("2", "2"), "3"), circle(("5", "6"), "3"));

    assert_eq!(from.distance(to), FixedU64::ZERO);
}

#[test]
fn range_past_coordinate_space_reaches_every_shape() {
    // The radius 1 and the range together reach 2³² + (2⁶⁴ − 1) raw bits,
    // whose square passes 2¹²⁸: the reach saturates and holds every gap.
    let (from, to) = (circle(("0.5", "0.5"), "1"), Shape::cell(utils::nav(60, 60)));

    assert!(from.within(to, FixedU64::MAX));
}

//
// ─── Nearest point ────────────────────────────────────────────────────────────
//

#[test]
fn box_nearest_point_clamps_onto_box() {
    let shape = boxed(("5", "5"), ("7", "6"));

    assert_eq!(
        shape.nearest_point(utils::point("9", "2")),
        utils::point("7", "5")
    );
    assert_eq!(
        shape.nearest_point(utils::point("6", "5.5")),
        utils::point("6", "5.5")
    );
}

#[test]
fn circle_nearest_point_lies_on_its_edge_toward_point() {
    let shape = circle(("2", "2"), "2.5");

    // (6, 5) lies 4 and 3 off the middle, 5 away: the edge point is half the
    // way, (2 + 4/2, 2 + 3/2).
    assert_eq!(
        shape.nearest_point(utils::point("6", "5")),
        utils::point("4", "3.5")
    );
    // A point inside is its own nearest.
    assert_eq!(
        shape.nearest_point(utils::point("2.5", "1.5")),
        utils::point("2.5", "1.5")
    );
}

#[test]
fn circle_nearest_point_lies_on_its_edge_toward_lower_point() {
    let shape = circle(("6", "5"), "2.5");

    // (2, 2) lies 4 and 3 below the middle, 5 away: the edge point is half the
    // way back, (6 − 4/2, 5 − 3/2).
    assert_eq!(
        shape.nearest_point(utils::point("2", "2")),
        utils::point("4", "3.5")
    );
}

//
// ─── Moving a shape ───────────────────────────────────────────────────────────
//

#[test]
fn box_moved_to_middle_keeps_its_size() {
    // A 2×2 box onto (3, 4.5): from 3 − 1, 4.5 − 1 to 3 + 1, 4.5 + 1.
    assert_eq!(
        boxed(("5.6", "5"), ("7.6", "7")).moved_to(utils::point("3", "4.5")),
        boxed(("2", "3.5"), ("4", "5.5"))
    );
}

#[test]
#[should_panic(expected = "a box moved to a middle stays clear of the origin")]
fn box_moved_across_origin_panics() {
    // A 2×2 box onto (0.5, 3) would reach x = 0.5 − 1 = −0.5.
    boxed(("5", "5"), ("7", "7")).moved_to(utils::point("0.5", "3"));
}

#[test]
#[should_panic(expected = "a box moved to a middle fits the coordinate space")]
fn box_moved_across_far_end_panics() {
    // A 2×2 box onto x = MAX − 0.5 would reach MAX − 0.5 + 1, past MAX.
    let middle = FixedUVec2::new(FixedU64::MAX - utils::fixed("0.5"), utils::fixed("3"));
    boxed(("5", "5"), ("7", "7")).moved_to(middle);
}

#[test]
fn circle_moved_to_middle_keeps_its_radius() {
    assert_eq!(
        circle(("5.5", "5.5"), "0.75").moved_to(utils::point("2", "8.25")),
        circle(("2", "8.25"), "0.75")
    );
}

//
// ─── Reach around a body ──────────────────────────────────────────────────────
//

#[test]
fn range_three_around_one_cell_reaches_sixty_cells() {
    // A cell d off on an axis is d − 1 away there. The 7×7 block, d ≤ 3, lies
    // within 2√2 ≈ 2.83: 49 cells. At d = 4 the gap is 3, kept only where
    // the other axis is 0 away (d ≤ 1): 3 cells on each of 4 sides, 12. In
    // all 61, 60 with the cell itself left out.
    let center = Shape::cell(utils::nav(20, 20));
    let reached = cells_within(center, FixedU64::from_num(3));

    assert_eq!(reached.len(), 61);
    assert_eq!(
        reached
            .iter()
            .filter(|&&cell| cell != utils::nav(20, 20))
            .count(),
        60
    );
}

#[test]
fn range_two_around_three_cell_body_reaches_sixty_cells() {
    // A 3×3 on 20–22. On an axis 5 columns lie 0 away (19 to 23), 2 lie 1
    // away and 2 lie 2 away. Kept where gx² + gy² ≤ 4: 0 and 0, 5 · 5 = 25;
    // 0 and 1 either way, 2 · 5 · 2 = 20; 1 and 1, 2 · 2 = 4; 0 and 2 either
    // way, 2 · 5 · 2 = 20 — 69, 60 besides its own 9.
    let body = boxed(("20", "20"), ("23", "23"));
    let reached = cells_within(body, FixedU64::from_num(2));

    assert_eq!(reached.len() - 9, 60);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The box from `min` to `max`, written as decimals.
fn boxed(min: (&str, &str), max: (&str, &str)) -> Shape {
    Shape::Box(FixedURect::new(
        utils::point(min.0, min.1),
        utils::point(max.0, max.1),
    ))
}

/// The circle of `radius` around `middle`, written as decimals.
fn circle(middle: (&str, &str), radius: &str) -> Shape {
    Shape::Circle {
        middle: utils::point(middle.0, middle.1),
        radius: utils::fixed(radius),
    }
}

/// Every cell of a 41×41 map whose gap to `shape` is at most `range`.
fn cells_within(shape: Shape, range: FixedU64) -> Vec<CellPos> {
    CellRect::new(utils::nav(0, 0), CellSize::new(41, 41))
        .cells()
        .filter(|&cell| Shape::cell(cell).within(shape, range))
        .collect()
}
