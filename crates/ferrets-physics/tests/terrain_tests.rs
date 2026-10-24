//! Bodies against standing terrain: overlap checks and sliding commits.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedI64, FixedU64};
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
        utils::position("3", "3"),
        FixedU64::from_num(0.5),
    ));
}

#[test]
fn body_reaching_into_wall_does_not_fit() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(!terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::position("4.7", "5"),
        FixedU64::from_num(0.5),
    ));
}

/// Touching a wall's boundary exactly is not entering it.
#[test]
fn body_touching_wall_boundary_fits() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::position("4", "5"),
        FixedU64::from_num(0.5),
    ));
}

/// Terrain the body's shape does not take refuses it as a wall does.
#[test]
fn body_reaching_into_terrain_off_its_shape_does_not_fit() {
    let grid = water_grid(&[(5, 5)]);

    assert!(!terrain::body_fits(
        &grid,
        utils::ground_shape(),
        utils::position("4.7", "5"),
        FixedU64::lit("0.5"),
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
        utils::position("3.9", "4.9"),
        utils::position("4.2", "5.2"),
        FixedU64::from_num(0.5),
    );

    assert_eq!(committed, utils::position("3.9", "5.2"));
}

/// A step wanted straight into a wall commits nothing — the caller reads no
/// movement out of a wanted one as "walled off".
#[test]
fn step_into_wall_stays_put() {
    let grid = walled_grid(&[(5, 4), (5, 5), (5, 6)]);
    let position = utils::position("4", "5");

    let committed = terrain::slide_toward(
        &grid,
        utils::ground_shape(),
        position,
        utils::position("4.3", "5"),
        FixedU64::from_num(0.5),
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
        utils::position("3.9", "4.9"),
        utils::position("4.2", "5.2"),
        FixedU64::lit("0.5"),
    );

    assert_eq!(committed, utils::position("3.9", "5.2"));
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
        utils::position("3.9", "4.9"),
        utils::position("4.2", "5.2"),
        FixedU64::lit("0.5"),
    );

    assert_eq!(committed, utils::position("4.2", "5.2"));
}

//
// ─── Displacement ─────────────────────────────────────────────────────────────
//

#[test]
fn displacement_applies_signed_pushes() {
    assert_eq!(
        terrain::displaced(
            utils::position("2", "2"),
            FixedI64::from_num(0.5),
            FixedI64::from_num(-0.25),
        ),
        utils::position("2.5", "1.75")
    );
}

#[test]
fn displacement_saturates_at_origin() {
    assert_eq!(
        terrain::displaced(
            utils::position("1", "1"),
            FixedI64::from_num(-2),
            FixedI64::ZERO,
        ),
        utils::position("0", "1")
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
    let desired = utils::position("4.4", "5");
    assert_eq!(
        terrain::slide_toward(
            &grid,
            utils::ground_shape(),
            utils::position("4.7", "5"),
            desired,
            FixedU64::from_num(0.5),
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
            utils::position("4.7", "5"),
            utils::position("4.9", "5.3"),
            FixedU64::from_num(0.5),
        ),
        utils::position("4.9", "5")
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
            utils::position(from.0, from.1),
            utils::position(to.0, to.1),
            FixedU64::lit("0.5"),
        )
    };

    // At (3, 0) a step to 3.3 reaches into the plateau's (4, 0) at 2.
    assert_eq!(
        slide(walker, ("3", "0"), ("3.3", "0")),
        utils::position("3", "0")
    );
    assert_eq!(
        slide(strider, ("3", "0"), ("3.3", "0")),
        utils::position("3.3", "0")
    );
    // At the ramp's (3, 1) the same step crosses its rising edge.
    assert_eq!(
        slide(walker, ("3", "1"), ("3.3", "1")),
        utils::position("3.3", "1")
    );
}

/// A small body stepping its center over a cliff edge in one stride, its
/// circle never overlapping both sides, still has to cross the edge.
#[test]
fn small_body_never_steps_its_center_across_cliff() {
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
        utils::position("3.4", "0"),
        utils::position("3.9", "0"),
        FixedU64::lit("0.1"),
    );

    assert_eq!(committed, utils::position("3.4", "0"));
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
    // At 3.3 the center lies at 3.8 in (3, 0) at 1, and a circle of 0.5
    // spans 3.3 to 4.3, into the plateau's (4, 0) at 2.
    let fits = |crossing| {
        terrain::body_fits(
            &grid,
            shape(crossing),
            utils::position("3.3", "0"),
            FixedU64::lit("0.5"),
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
        grid.set_occupied_by(utils::GROUND, CellPos::new(x, y), true);
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
