//! Bodies against standing terrain: overlap checks and sliding commits.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::FixedI64;
use ferrets_nav_grid::{
    cliff_crossing::CliffCrossing,
    mover_shape::MoverShape,
    nav_grid::NavGrid,
    relief::Relief,
    slope::{Direction, Slope},
    terrain_mask::TerrainMask,
};
use ferrets_physics::terrain;

//
// ─── Fit ──────────────────────────────────────────────────────────────────────
//

#[test]
fn body_fits_open_ground() {
    let grid = walled_grid(&[]);

    assert!(terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::point("3.5", "3.5"),
        utils::cells("0.5"),
    ));
}

#[test]
fn body_reaching_into_wall_does_not_fit() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(!terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::point("5.2", "5.5"),
        utils::cells("0.5"),
    ));
}

/// Touching a wall's boundary exactly is not entering it.
#[test]
fn body_touching_wall_boundary_fits() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::point("4.5", "5.5"),
        utils::cells("0.5"),
    ));
}

/// Terrain the body's shape does not take refuses it as a wall does.
#[test]
fn body_reaching_into_terrain_off_its_shape_does_not_fit() {
    let grid = water_grid(&[(5, 5)]);

    assert!(!terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::point("5.2", "5.5"),
        utils::cells("0.5"),
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

    let committed = terrain::slide_toward(
        &grid,
        utils::ground_shape(),
        utils::point("4.4", "5.4"),
        utils::point("4.7", "5.7"),
        utils::cells("0.5"),
    );

    assert_eq!(committed, utils::point("4.4", "5.7"));
}

/// A step wanted straight into a wall commits nothing — the caller reads no
/// movement out of a wanted one as "walled off".
#[test]
fn step_into_wall_stays_put() {
    let grid = walled_grid(&[(5, 4), (5, 5), (5, 6)]);
    let position = utils::point("4.5", "5.5");

    let committed = terrain::slide_toward(
        &grid,
        utils::ground_shape(),
        position,
        utils::point("4.8", "5.5"),
        utils::cells("0.5"),
    );

    assert_eq!(committed, position);
}

/// The same diagonal step into a column of water: the x advance is refused
/// as by a wall, the y advance survives.
#[test]
fn blocked_axis_drops_against_terrain_off_its_shape() {
    let grid = water_grid(&[(5, 4), (5, 5), (5, 6)]);

    let committed = terrain::slide_toward(
        &grid,
        utils::ground_shape(),
        utils::point("4.4", "5.4"),
        utils::point("4.7", "5.7"),
        utils::cells("0.5"),
    );

    assert_eq!(committed, utils::point("4.4", "5.7"));
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

    let committed = terrain::slide_toward(
        &grid,
        wading,
        utils::point("4.4", "5.4"),
        utils::point("4.7", "5.7"),
        utils::cells("0.5"),
    );

    assert_eq!(committed, utils::point("4.7", "5.7"));
}

//
// ─── Displacement ─────────────────────────────────────────────────────────────
//

#[test]
fn displacement_applies_signed_pushes() {
    // x: 2.5 + 0.5 = 3; y: 2.5 − 0.25 = 2.25.
    assert_eq!(
        terrain::displaced(
            utils::location("2.5", "2.5", CellSize::ONE),
            utils::signed_cells("0.5"),
            utils::signed_cells("-0.25"),
            utils::MAP_SIZE,
        ),
        utils::point("3", "2.25")
    );
}

#[test]
fn displacement_holds_body_clear_of_origin() {
    // A 1×1 pushed 2 west from x 1.5 stops with its middle at 1/2, its edge
    // on the origin; a 2×2 at 2/2 = 1.
    assert_eq!(
        terrain::displaced(
            utils::location("1.5", "1.5", CellSize::ONE),
            FixedI64::from_num(-2),
            FixedI64::ZERO,
            utils::MAP_SIZE,
        ),
        utils::point("0.5", "1.5")
    );
    assert_eq!(
        terrain::displaced(
            utils::location("2", "2", CellSize::new(2, 2)),
            FixedI64::from_num(-2),
            FixedI64::ZERO,
            utils::MAP_SIZE,
        ),
        utils::point("1", "2")
    );
}

#[test]
fn displacement_holds_body_short_of_far_edge() {
    // A 2×2 pushed 2 south from y 62 on a 64-high map stops with its middle
    // at 64 − 2/2 = 63, its edge on the map's.
    assert_eq!(
        terrain::displaced(
            utils::location("10", "62", CellSize::new(2, 2)),
            FixedI64::ZERO,
            FixedI64::from_num(2),
            utils::MAP_SIZE,
        ),
        utils::point("10", "63")
    );
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
    assert!(!terrain::body_fits(
        &grid,
        shape,
        utils::point("0.5", "4"),
        utils::cells("0.4"),
    ));
    // At x 1 it spans 0–2, its edge on the origin's.
    assert!(terrain::body_fits(
        &grid,
        shape,
        utils::point("1", "4"),
        utils::cells("0.4"),
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
        terrain::slide_toward(
            &grid,
            utils::ground_shape(),
            utils::point("0.6", "3.5"),
            utils::point("0.3", "3.8"),
            utils::cells("0.5"),
        ),
        utils::point("0.6", "3.8")
    );
    assert_eq!(
        terrain::slide_toward(
            &grid,
            utils::ground_shape(),
            utils::point("7.4", "3.5"),
            utils::point("7.7", "3.8"),
            utils::cells("0.5"),
        ),
        utils::point("7.4", "3.8")
    );
}

//
// ─── Draining a clip ──────────────────────────────────────────────────────────
//

/// A building raised against a standing body leaves its circle clipping the
/// static footprint; a step out of the clip must commit, or the body is
/// frozen where it stands forever.
#[test]
fn clipped_body_walks_out_of_wall() {
    let grid = walled_grid(&[(5, 5)]);

    // Clipping the wall's west face; a step further west drains the clip.
    let desired = utils::point("4.9", "5.5");
    assert_eq!(
        terrain::slide_toward(
            &grid,
            utils::ground_shape(),
            utils::point("5.2", "5.5"),
            desired,
            utils::cells("0.5"),
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
        terrain::slide_toward(
            &grid,
            utils::ground_shape(),
            utils::point("5.2", "5.5"),
            utils::point("5.4", "5.8"),
            utils::cells("0.5"),
        ),
        utils::point("5.4", "5.5")
    );
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
        terrain::slide_toward(
            &grid,
            shape,
            utils::point(from.0, from.1),
            utils::point(to.0, to.1),
            utils::cells("0.5"),
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
    let committed = terrain::slide_toward(
        &grid,
        walker,
        utils::point("3.9", "0.5"),
        utils::point("4.4", "0.5"),
        utils::cells("0.1"),
    );

    assert_eq!(committed, utils::point("3.9", "0.5"));
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
        terrain::body_fits(
            &grid,
            shape(crossing),
            utils::point("3.8", "0.5"),
            utils::cells("0.5"),
        )
    };

    assert!(!fits(CliffCrossing::Ramp));
    assert!(fits(CliffCrossing::Cross));
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
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
