//! `Location`: a body's place, anchor, footprint, bounds and snapping.

mod utils;

use ferrets_geometry::{cell_rect::CellRect, cell_size::CellSize, location::Location};
use ferrets_math::fixed_urect::FixedURect;

/// The map every test places its bodies on.
const MAP_SIZE: CellSize = CellSize::new(16, 16);

//
// ─── Place at an anchor ───────────────────────────────────────────────────────
//

#[test]
fn single_cell_sits_on_its_cell_middle() {
    // (5, 5) + 1/2 on each axis.
    assert_eq!(
        Location::at(utils::nav(5, 5), CellSize::ONE, MAP_SIZE).middle(),
        utils::point("5.5", "5.5")
    );
}

#[test]
fn even_body_sits_on_cell_corner() {
    // (5, 5) + 2/2 on each axis.
    assert_eq!(
        Location::at(utils::nav(5, 5), CellSize::new(2, 2), MAP_SIZE).middle(),
        utils::world(6, 6)
    );
}

#[test]
fn uneven_body_sits_per_axis() {
    // x: 4 + 3/2 = 5.5; y: 2 + 2/2 = 3.
    assert_eq!(
        Location::at(utils::nav(4, 2), CellSize::new(3, 2), MAP_SIZE).middle(),
        utils::point("5.5", "3")
    );
}

#[test]
#[should_panic(expected = "a body lies wholly on the map")]
fn anchor_past_far_edge_panics() {
    // A 2×2 anchored at (15, 3) covers column 16, past a 16-wide map.
    Location::at(utils::nav(15, 3), CellSize::new(2, 2), MAP_SIZE);
}

#[test]
fn anchor_past_far_edge_is_clamped_onto_map() {
    // A 2×2 at (15, 15) would sit at (16, 16); held at 16 − 2/2 = 15, its
    // anchor (14, 14). A 3×1 at (15, 0) would sit at (16.5, 0.5); held at
    // 16 − 3/2 = 14.5, its anchor (13, 0).
    let even = Location::at_clamped(utils::nav(15, 15), CellSize::new(2, 2), MAP_SIZE);
    assert_eq!(even.middle(), utils::world(15, 15));
    assert_eq!(even.anchor(), utils::nav(14, 14));
    let uneven = Location::at_clamped(utils::nav(15, 0), CellSize::new(3, 1), MAP_SIZE);
    assert_eq!(uneven.middle(), utils::point("14.5", "0.5"));
    assert_eq!(uneven.anchor(), utils::nav(13, 0));
}

#[test]
fn anchor_on_map_is_not_clamped() {
    // A 2×2 at (14, 14) already lies on the map: the place anchored there.
    assert_eq!(
        Location::at_clamped(utils::nav(14, 14), CellSize::new(2, 2), MAP_SIZE),
        Location::at(utils::nav(14, 14), CellSize::new(2, 2), MAP_SIZE)
    );
}

//
// ─── On the map ───────────────────────────────────────────────────────────────
//

#[test]
fn body_touching_both_edges_lies_on_map() {
    // A 2×2 needs its middle within 2/2 = 1 of either edge: 1 and 16 − 1 = 15.
    let low = Location::new(utils::world(1, 1), CellSize::new(2, 2), MAP_SIZE);
    let high = Location::new(utils::world(15, 15), CellSize::new(2, 2), MAP_SIZE);
    assert_eq!(low.footprint().origin, utils::nav(0, 0));
    assert_eq!(high.footprint().origin, utils::nav(14, 14));
}

#[test]
#[should_panic(expected = "a body lies wholly on the map")]
fn body_reaching_before_origin_panics() {
    // A 2×2 at x 0.4: its low edge 0.4 − 1 lies before the origin.
    Location::new(utils::point("0.4", "5"), CellSize::new(2, 2), MAP_SIZE);
}

#[test]
#[should_panic(expected = "a body lies wholly on the map")]
fn body_reaching_past_far_edge_panics() {
    // A 1×1 at y 15.6: its high edge 15.6 + 1/2 = 16.1 lies past the edge.
    Location::new(utils::point("3", "15.6"), CellSize::ONE, MAP_SIZE);
}

#[test]
fn body_off_map_is_clamped_onto_it() {
    // A 2×2 needs its middle within [1, 15] on each axis: x 0.25 rises to
    // 1, y 15.5 falls to 15.
    assert_eq!(
        Location::clamped(utils::point("0.25", "15.5"), CellSize::new(2, 2), MAP_SIZE).middle(),
        utils::world(1, 15)
    );
    // A 1×1 needs [1/2, 15.5]: x 0.25 rises to 0.5, y 15.5 already lies on it.
    assert_eq!(
        Location::clamped(utils::point("0.25", "15.5"), CellSize::ONE, MAP_SIZE).middle(),
        utils::point("0.5", "15.5")
    );
}

#[test]
fn body_on_map_is_not_clamped() {
    // A 2×2 on a 16-wide map needs [2/2, 16 − 2/2] = [1, 15] on each axis:
    // x 4.3 and y 1 already lie in it.
    let middle = utils::point("4.3", "1");
    assert_eq!(
        Location::clamped(middle, CellSize::new(2, 2), MAP_SIZE).middle(),
        middle
    );
}

#[test]
#[should_panic(expected = "a body fits on the map")]
fn body_wider_than_map_cannot_be_clamped() {
    Location::clamped(utils::world(8, 8), CellSize::new(17, 1), MAP_SIZE);
}

//
// ─── Anchor ───────────────────────────────────────────────────────────────────
//

#[test]
fn anchored_place_anchors_back_at_its_cell() {
    for size in [CellSize::ONE, CellSize::new(2, 2), CellSize::new(3, 2)] {
        let anchor = utils::nav(7, 4);
        assert_eq!(
            Location::at(anchor, size, MAP_SIZE).anchor(),
            anchor,
            "{size:?}"
        );
    }
}

#[test]
fn anchor_rounds_low_edge_to_nearest_cell() {
    // A 2×2 at x 6.3: low edge 6.3 − 1 = 5.3 rounds to 5; at x 6.6: 5.6 rounds
    // to 6. Y 6: low edge 5.
    assert_eq!(
        Location::new(utils::point("6.3", "6"), CellSize::new(2, 2), MAP_SIZE).anchor(),
        utils::nav(5, 5)
    );
    assert_eq!(
        Location::new(utils::point("6.6", "6"), CellSize::new(2, 2), MAP_SIZE).anchor(),
        utils::nav(6, 5)
    );
}

#[test]
fn anchor_rounds_half_up() {
    // A 1×1 at x 6: low edge 5.5 lies halfway between 5 and 6 and rounds up
    // to 6. Y 5.5: low edge 5.
    assert_eq!(
        Location::new(utils::point("6", "5.5"), CellSize::ONE, MAP_SIZE).anchor(),
        utils::nav(6, 5)
    );
}

//
// ─── Footprint ────────────────────────────────────────────────────────────────
//

#[test]
fn footprint_spans_size_from_anchor() {
    // A 2×2 at (6.3, 6): anchored at (5, 5) (low edges 5.3 and 5 round to
    // 5), covering cells 5–6 on each axis.
    assert_eq!(
        Location::new(utils::point("6.3", "6"), CellSize::new(2, 2), MAP_SIZE).footprint(),
        CellRect::new(utils::nav(5, 5), CellSize::new(2, 2))
    );
}

//
// ─── Bounds ───────────────────────────────────────────────────────────────────
//

#[test]
fn bounds_span_half_size_either_side() {
    // x: 6.3 ∓ 1 = 5.3–7.3; y: 6 ∓ 1 = 5–7.
    assert_eq!(
        Location::new(utils::point("6.3", "6"), CellSize::new(2, 2), MAP_SIZE).bounds(),
        FixedURect::new(utils::point("5.3", "5"), utils::point("7.3", "7"))
    );
}

//
// ─── Snapping ─────────────────────────────────────────────────────────────────
//

#[test]
fn snapped_lines_up_on_cell_counted_on() {
    // A 2×2 at x 6.3: anchored at 5 (5.3 rounds to 5), snapped to 5 + 2/2 = 6;
    // at x 6.6: anchored at 6, snapped to 7. Y 6 is already lined up.
    assert_eq!(
        Location::new(utils::point("6.3", "6"), CellSize::new(2, 2), MAP_SIZE)
            .snapped()
            .middle(),
        utils::world(6, 6)
    );
    assert_eq!(
        Location::new(utils::point("6.6", "6"), CellSize::new(2, 2), MAP_SIZE)
            .snapped()
            .middle(),
        utils::world(7, 6)
    );
}

#[test]
fn snapped_toward_breaks_tie_toward_given_cell() {
    // A 2×2 at (5.5, 5.5): low edges 5.5 − 1 = 4.5 lie halfway between 4
    // and 5. Toward (5, 5) it takes 5 on each axis, snapped middle 5 + 1 = 6;
    // toward (3, 7) it takes 4 on x (middle 4 + 1 = 5) and 5 on y.
    let between = Location::new(utils::point("5.5", "5.5"), CellSize::new(2, 2), MAP_SIZE);
    assert_eq!(
        between.snapped_toward(utils::nav(5, 5)).middle(),
        utils::world(6, 6)
    );
    assert_eq!(
        between.snapped_toward(utils::nav(3, 7)).middle(),
        utils::world(5, 6)
    );
}

#[test]
fn snapped_toward_rounds_without_tie() {
    // A 2×2 at x 6.3: low edge 5.3 is no tie and rounds to 5, snapped middle 6,
    // whichever cell it is told to lean toward.
    let off = Location::new(utils::point("6.3", "6"), CellSize::new(2, 2), MAP_SIZE);
    assert_eq!(
        off.snapped_toward(utils::nav(9, 5)).middle(),
        utils::world(6, 6)
    );
}

#[test]
fn anchored_place_is_not_mid_crossing() {
    // 5 + 1/2 = 5.5 for a 1×1; 5 + 2/2 = 6 for a 2×2.
    assert!(!Location::new(utils::point("5.5", "5.5"), CellSize::ONE, MAP_SIZE).is_mid_crossing());
    assert!(!Location::new(utils::world(6, 6), CellSize::new(2, 2), MAP_SIZE).is_mid_crossing());
}

#[test]
fn place_between_cells_is_mid_crossing() {
    // A 1×1 crossing from (5, 5) to (6, 5), a quarter of the way.
    assert!(Location::new(utils::point("5.75", "5.5"), CellSize::ONE, MAP_SIZE).is_mid_crossing());
}
