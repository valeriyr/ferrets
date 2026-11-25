//! The ground a body may not enter: terrain, the map's edge and bodies.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, shape::Shape};
use ferrets_math::{FixedU64, fixed_urect::FixedURect, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    relief::Relief,
    slope::{Direction, Slope},
    terrain_mask::TerrainMask,
};
use ferrets_physics::{
    ground::{self, Ground},
    motion,
};

//
// ─── Fit ──────────────────────────────────────────────────────────────────────
//

#[test]
fn body_fits_open_ground() {
    let grid = walled_grid(&[]);

    assert!(ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("3.5", "3.5"), utils::cells("0.5"))
    ));
}

#[test]
fn body_reaching_into_wall_does_not_fit() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(!ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("5.2", "5.5"), utils::cells("0.5"))
    ));
}

/// Touching a wall's boundary exactly is not entering it.
#[test]
fn body_touching_wall_boundary_fits() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.5", "5.5"), utils::cells("0.5"))
    ));
}

/// A body of no radius stands on the cell its middle lies in, and fits only
/// where that cell is clear.
#[test]
fn body_of_no_radius_fits_only_on_clear_cell() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(!ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("5.5", "5.5"), utils::cells("0"))
    ));
    assert!(ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.5", "5.5"), utils::cells("0"))
    ));
}

/// Terrain the body's shape does not take refuses it as a wall does.
#[test]
fn body_reaching_into_terrain_off_its_shape_does_not_fit() {
    let grid = water_grid(&[(5, 5)]);

    assert!(!ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("5.2", "5.5"), utils::cells("0.5"))
    ));
}

/// A box body is judged by its corners: one whose corner reaches into a
/// diagonal wall does not fit where a circle of the same middle does.
#[test]
fn box_body_reaching_into_wall_by_its_corner_does_not_fit() {
    let grid = walled_grid(&[(5, 4)]);

    // Middle (4.6, 5.4): a circle of 0.5 lies √(0.4² + 0.4²) ≈ 0.57 from
    // the wall's corner (5, 5), clear of it; the 1×1 box spans 4.1–5.1 by
    // 4.9–5.9, into (5, 4).
    assert!(ground::body_fits(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.6", "5.4"), utils::cells("0.5")),
    ));
    assert!(!ground::body_fits(
        &grid,
        utils::ground_shape(),
        boxed(utils::point("4.6", "5.4")),
    ));
}

//
// ─── Slide ────────────────────────────────────────────────────────────────────
//

/// A diagonal step into a wall keeps its unblocked axis:
///
///   . # .
///   ○↘# .    the x advance is walled, the y advance survives
///   . # .
#[test]
fn blocked_axis_drops_and_slide_keeps_other() {
    let grid = walled_grid(&[(5, 4), (5, 5), (5, 6)]);

    let reached = step(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.4", "5.4"), utils::cells("0.5")),
        utils::point("4.7", "5.7"),
    );

    assert_eq!(reached, utils::point("4.4", "5.7"));
}

/// A step wanted straight into a wall moves nothing — the caller reads no
/// movement out of a wanted one as "walled off".
#[test]
fn step_into_wall_stays_put() {
    let grid = walled_grid(&[(5, 4), (5, 5), (5, 6)]);
    let position = utils::point("4.5", "5.5");

    let reached = step(
        &grid,
        utils::ground_shape(),
        circle(position, utils::cells("0.5")),
        utils::point("4.8", "5.5"),
    );

    assert_eq!(reached, position);
}

/// The same diagonal step into a column of water: the x advance is refused
/// as by a wall, the y advance survives.
#[test]
fn blocked_axis_drops_against_terrain_off_its_shape() {
    let grid = water_grid(&[(5, 4), (5, 5), (5, 6)]);

    let reached = step(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.4", "5.4"), utils::cells("0.5")),
        utils::point("4.7", "5.7"),
    );

    assert_eq!(reached, utils::point("4.4", "5.7"));
}

/// A shape that takes the water too steps onto it whole.
#[test]
fn shape_over_water_slides_onto_it() {
    let grid = water_grid(&[(5, 4), (5, 5), (5, 6)]);
    let wading = MoverShape::point(
        TerrainMask::from(utils::LAND) | utils::WATER,
        utils::GROUND,
        None,
    );

    let reached = step(
        &grid,
        wading,
        circle(utils::point("4.4", "5.4"), utils::cells("0.5")),
        utils::point("4.7", "5.7"),
    );

    assert_eq!(reached, utils::point("4.7", "5.7"));
}

/// A body of no radius is the cell its middle lies in: a diagonal step from
/// (4.5, 5.5) to (5.2, 5.8) would put it in water at (5, 5), so the x advance
/// drops and the y advance survives.
#[test]
fn body_of_no_radius_is_stopped_by_water() {
    let grid = water_grid(&[(5, 4), (5, 5), (5, 6)]);

    let reached = step(
        &grid,
        utils::ground_shape(),
        circle(utils::point("4.5", "5.5"), utils::cells("0")),
        utils::point("5.2", "5.8"),
    );

    assert_eq!(reached, utils::point("4.5", "5.8"));
}

/// A body of radius 0.7 moves as its 2×2 square. Stepping from (5, 5) to
/// (5.5, 5.5) beside water at (6, 6), its circle would clear the water by
/// √(0.5² + 0.5²) − 0.7 ≈ 0.007, but its square, anchored at
/// (5.5 − 1 + 0.5, 5.5 − 1 + 0.5) = (5, 5), would cover it; the x advance
/// alone, anchored at (5, 4), keeps its square on land:
///
///   . . . .     y 4
///   . ○→. .     y 5
///   . . . ≈     y 6, water at (6, 6)
#[test]
fn step_keeps_body_square_off_ground_its_circle_clears() {
    let grid = water_grid(&[(6, 6)]);
    let stout = MoverShape::new(
        TerrainMask::from(utils::LAND),
        utils::GROUND,
        utils::GROUND,
        CellSize::new(2, 2),
        None,
    );

    let reached = step(
        &grid,
        stout,
        circle(utils::point("5", "5"), utils::cells("0.7")),
        utils::point("5.5", "5.5"),
    );

    assert_eq!(reached, utils::point("5.5", "5"));
}

//
// ─── Map edges ────────────────────────────────────────────────────────────────
//

#[test]
fn body_reaching_before_origin_does_not_fit() {
    // A 2×2 shape with a circle of radius 0.4 at x 0.5: the circle stays on
    // the map, but the body spans 0.5 ∓ 1, past the origin.
    let grid = walled_grid(&[]);
    let shape = MoverShape {
        size: CellSize::new(2, 2),
        ..utils::ground_shape()
    };
    assert!(!ground::body_fits(
        &grid,
        shape,
        circle(utils::point("0.5", "4"), utils::cells("0.4"))
    ));
    // At x 1 it spans 0–2, its edge on the origin's.
    assert!(ground::body_fits(
        &grid,
        shape,
        circle(utils::point("1", "4"), utils::cells("0.4"))
    ));
}

#[test]
fn step_past_either_edge_slides_along_it() {
    // A 1×1 at (0.6, 3.5) stepping to (0.3, 3.8) would reach 0.3 − 0.5 past
    // the origin: x drops and the step keeps y. On the 8-wide grid, one at
    // (7.4, 3.5) stepping to (7.7, 3.8) would reach 7.7 + 0.5 = 8.2 past the
    // far edge: the same.
    let grid = walled_grid(&[]);
    assert_eq!(
        step(
            &grid,
            utils::ground_shape(),
            circle(utils::point("0.6", "3.5"), utils::cells("0.5")),
            utils::point("0.3", "3.8")
        ),
        utils::point("0.6", "3.8")
    );
    assert_eq!(
        step(
            &grid,
            utils::ground_shape(),
            circle(utils::point("7.4", "3.5"), utils::cells("0.5")),
            utils::point("7.7", "3.8")
        ),
        utils::point("7.4", "3.8")
    );
}

/// A box body slides as a box: the step its corner would take into a
/// diagonal wall drops the axis that carries the corner there.
#[test]
fn box_body_slides_by_its_corner() {
    let grid = walled_grid(&[(5, 4)]);

    // From (4.5, 5.5) to (4.75, 5.25) the box would span 4.25–5.25 by
    // 4.75–5.75, into (5, 4); the axes tie at 0.25 and x is kept: at
    // (4.75, 5.5) it spans 5–6 in y, clear of row 4.
    assert_eq!(
        step(
            &grid,
            utils::ground_shape(),
            boxed(utils::point("4.5", "5.5")),
            utils::point("4.75", "5.25"),
        ),
        utils::point("4.75", "5.5")
    );
}

//
// ─── Draining a clip ──────────────────────────────────────────────────────────
//

/// A footprint set down against a standing body leaves its circle clipping
/// the static footprint; a step out of the clip must commit, or the body is
/// frozen where it stands forever.
#[test]
fn clipped_body_walks_out_of_wall() {
    let grid = walled_grid(&[(5, 5)]);

    // Clipping the wall's west face; a step further west drains the clip.
    let desired = utils::point("4.9", "5.5");
    assert_eq!(
        step(
            &grid,
            utils::ground_shape(),
            circle(utils::point("5.2", "5.5"), utils::cells("0.5")),
            desired
        ),
        desired
    );
}

#[test]
fn clipped_body_cannot_clip_deeper() {
    let grid = walled_grid(&[(5, 5), (5, 6)]);

    // Clipping (5, 5) already; stepping south-east would newly overlap
    // (5, 6), so only the along-x axis (which keeps the existing clip and
    // adds nothing) commits.
    assert_eq!(
        step(
            &grid,
            utils::ground_shape(),
            circle(utils::point("5.2", "5.5"), utils::cells("0.5")),
            utils::point("5.4", "5.8")
        ),
        utils::point("5.4", "5.5")
    );
}

/// A clipped square body may step to a new anchor that keeps the blocked
/// cell it already overlaps, and no new one: a radius-0.7 body (a 2×2
/// square) at (5, 5) holds (4..=5, 4..=5), clipping the wall at (5, 5);
/// aimed at (5.5, 5.5) its square would anchor at (5, 5) and newly take the
/// wall at (6, 6), so the step keeps its x axis alone, to (5.5, 5) — a new
/// anchor (5, 4) whose square (5..=6, 4..=5) keeps the clip and adds nothing.
#[test]
fn clipped_square_steps_to_new_anchor_keeping_its_clip() {
    let grid = walled_grid(&[(5, 5), (6, 6)]);
    let stout = MoverShape::new(
        TerrainMask::from(utils::LAND),
        utils::GROUND,
        utils::GROUND,
        CellSize::new(2, 2),
        None,
    );

    let reached = step(
        &grid,
        stout,
        circle(utils::point("5", "5"), utils::cells("0.7")),
        utils::point("5.5", "5.5"),
    );

    assert_eq!(reached, utils::point("5.5", "5"));
}

//
// ─── Levels ───────────────────────────────────────────────────────────────────
//

/// A body walking east at the cliff beside the ramp stops against it, one on
/// the ramp's upper cell climbs it, and a cliff walker steps up the cliff.
#[test]
fn body_climbs_only_where_ramp_rises_unless_it_walks_cliffs() {
    let grid = hill_grid();
    let walker = MoverShape::point(
        TerrainMask::from(utils::LAND),
        utils::GROUND,
        Some(CliffCrossing::Ramp),
    );
    let strider = MoverShape::point(
        TerrainMask::from(utils::LAND),
        utils::GROUND,
        Some(CliffCrossing::Cross),
    );
    let slide = |shape, from: (&str, &str), to: (&str, &str)| {
        step(
            &grid,
            shape,
            circle(utils::point(from.0, from.1), utils::cells("0.5")),
            utils::point(to.0, to.1),
        )
    };

    // At rest on (3, 0) a step to 3.8 reaches into the plateau's (4, 0) at 2.
    assert_eq!(
        slide(walker, ("3.5", "0.5"), ("3.8", "0.5")),
        utils::point("3.5", "0.5")
    );
    assert_eq!(
        slide(strider, ("3.5", "0.5"), ("3.8", "0.5")),
        utils::point("3.8", "0.5")
    );
    // On the ramp's (3, 1) the same step crosses its rising edge.
    assert_eq!(
        slide(walker, ("3.5", "1.5"), ("3.8", "1.5")),
        utils::point("3.8", "1.5")
    );
}

/// A small body stepping its center over a cliff edge in one stride, its
/// circle never overlapping both sides, still has to cross the edge.
#[test]
fn small_body_never_steps_its_middle_across_cliff() {
    let grid = hill_grid();
    let walker = MoverShape::point(
        TerrainMask::from(utils::LAND),
        utils::GROUND,
        Some(CliffCrossing::Ramp),
    );

    // Centered at 3.9 its circle of 0.1 spans 3.8 to 4.0, in (3, 0); at 4.4
    // it spans 4.3 to 4.5, wholly in the plateau's (4, 0) at 2.
    let reached = step(
        &grid,
        walker,
        circle(utils::point("3.9", "0.5"), utils::cells("0.1")),
        utils::point("4.4", "0.5"),
    );

    assert_eq!(reached, utils::point("3.9", "0.5"));
}

/// A body whose circle reaches across a cliff edge the map paints with no
/// band fits only if its cliff crossing joins the two cells.
#[test]
fn body_overlapping_cell_beyond_cliff_fits_only_if_it_crosses() {
    let grid = hill_grid();
    let shape = |crossing| {
        MoverShape::point(
            TerrainMask::from(utils::LAND),
            utils::GROUND,
            Some(crossing),
        )
    };
    // At 3.8 the middle lies in (3, 0) at 1, and a circle of 0.5 spans 3.3
    // to 4.3, into the plateau's (4, 0) at 2.
    let fits = |crossing| {
        ground::body_fits(
            &grid,
            shape(crossing),
            circle(utils::point("3.8", "0.5"), utils::cells("0.5")),
        )
    };

    assert!(!fits(CliffCrossing::Ramp));
    assert!(fits(CliffCrossing::Cross));
}

//
// ─── Obstacles ────────────────────────────────────────────────────────────────
//

/// A walker sinks into an obstacle only by ending deeper than it stands: 0.8
/// from a middle it touches at 1, ending 0.6 away sinks; ending 0.8 away on
/// another side, or 0.9 away still overlapping, does not.
#[test]
fn walker_sinks_only_by_ending_deeper() {
    let walker = utils::resting("4.2", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let obstacles = [obstacle];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    assert!(!ground.admits(utils::point("4.4", "5")));
    assert!(ground.admits(utils::point("5", "4.2")));
    assert!(ground.admits(utils::point("4.1", "5")));
}

/// A step that ends clear of the obstacle never sinks, nearer as it comes:
/// from 1.5 away to 1.2.
#[test]
fn step_ending_clear_never_sinks() {
    let walker = utils::resting("3.5", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let obstacles = [obstacle];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    assert!(ground.admits(utils::point("3.8", "5")));
}

/// Sinking is judged between circles alone.
#[test]
#[should_panic(expected = "contact is between circles")]
fn sinking_into_box_panics() {
    let walker = utils::resting("4.2", "5").shape();
    let obstacle = Shape::cell(CellPos::new(5, 5));

    let obstacles = [obstacle];
    let grid = utils::open_grid();

    Ground::new(&grid, utils::ground_shape(), walker, &obstacles).admits(utils::point("4.4", "5"));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// An 8×8 single-layer grid with the given cells statically walled.
fn walled_grid(walls: &[(u32, u32)]) -> NavGrid {
    let mut grid = NavGrid::new(8, 8, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);
    for &(x, y) in walls {
        grid.set_occupied(utils::GROUND, CellPos::new(x, y), true);
    }
    grid
}

/// An 8×8 grid of land whose `water` cells are water, nothing standing on it.
fn water_grid(water: &[(u32, u32)]) -> NavGrid {
    let mut grid = NavGrid::new(8, 8, |cell| {
        if water.contains(&(cell.x, cell.y)) {
            utils::WATER
        } else {
            utils::LAND
        }
    });
    grid.add_flat_layer(utils::GROUND, 1);
    grid
}

/// An 8×5 land grid: low ground at 1 to column 3, a plateau at 2 beyond, and
/// a ramp at (3, 1) and (3, 2) rising east onto it.
fn hill_grid() -> NavGrid {
    let mut grid = NavGrid::new(8, 5, |_| utils::LAND);
    grid.add_ranged_layer(utils::GROUND, 1, 2, |cell| Relief {
        level: if cell.x >= 4 { 2 } else { 1 },
        slope: match (cell.x, cell.y) {
            (3, 1) | (3, 2) => Some(Slope::Ramp(Direction::East)),
            _ => None,
        },
    });
    grid
}

/// The circle of `radius` around `middle`.
fn circle(middle: FixedUVec2, radius: FixedU64) -> Shape {
    Shape::Circle { middle, radius }
}

/// The 1×1 box around `middle`.
fn boxed(middle: FixedUVec2) -> Shape {
    Shape::Box(FixedURect::new(
        FixedUVec2::new(
            middle.x - utils::cells("0.5"),
            middle.y - utils::cells("0.5"),
        ),
        FixedUVec2::new(
            middle.x + utils::cells("0.5"),
            middle.y + utils::cells("0.5"),
        ),
    ))
}

/// Where a body whose circle is `body`, moving as `mover` over `grid`, gets
/// toward `desired` past the terrain alone.
fn step(grid: &NavGrid, mover: MoverShape, body: Shape, desired: FixedUVec2) -> FixedUVec2 {
    motion::advance(desired, &Ground::new(grid, mover, body, &[])).point(body.middle())
}
