//! Bodies against standing terrain: overlap checks and sliding commits.

mod utils;

use ferrets_geometry::cell_pos::CellPos;
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::NavGrid};
use ferrets_physics::terrain;

//
// ─── Fit ──────────────────────────────────────────────────────────────────────
//

#[test]
fn body_fits_open_ground() {
    let grid = walled_grid(&[]);

    assert!(terrain::body_fits(
        &grid,
        LayerMask::from(utils::GROUND),
        utils::position(3.0, 3.0),
        CellSize::ONE,
        FixedU64::from_num(0.5),
    ));
}

#[test]
fn body_reaching_into_wall_does_not_fit() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(!terrain::body_fits(
        &grid,
        LayerMask::from(utils::GROUND),
        utils::position(4.7, 5.0),
        CellSize::ONE,
        FixedU64::from_num(0.5),
    ));
}

/// Touching a wall's boundary exactly is not entering it.
#[test]
fn body_touching_wall_boundary_fits() {
    let grid = walled_grid(&[(5, 5)]);

    assert!(terrain::body_fits(
        &grid,
        LayerMask::from(utils::GROUND),
        utils::position(4.0, 5.0),
        CellSize::ONE,
        FixedU64::from_num(0.5),
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
        LayerMask::from(utils::GROUND),
        utils::position(3.9, 4.9),
        CellSize::ONE,
        utils::position(4.2, 5.2),
        FixedU64::from_num(0.5),
    );

    assert_eq!(committed, utils::position(3.9, 5.2));
}

/// A step wanted straight into a wall commits nothing — the caller reads no
/// movement out of a wanted one as "walled off".
#[test]
fn step_into_wall_stays_put() {
    let grid = walled_grid(&[(5, 4), (5, 5), (5, 6)]);
    let position = utils::position(4.0, 5.0);

    let committed = terrain::slide_toward(
        &grid,
        LayerMask::from(utils::GROUND),
        position,
        CellSize::ONE,
        utils::position(4.3, 5.0),
        FixedU64::from_num(0.5),
    );

    assert_eq!(committed, position);
}

//
// ─── Displacement ─────────────────────────────────────────────────────────────
//

#[test]
fn displacement_applies_signed_pushes() {
    assert_eq!(
        terrain::displaced(
            utils::position(2.0, 2.0),
            FixedI64::from_num(0.5),
            FixedI64::from_num(-0.25),
        ),
        utils::position(2.5, 1.75)
    );
}

#[test]
fn displacement_saturates_at_origin() {
    assert_eq!(
        terrain::displaced(
            utils::position(1.0, 1.0),
            FixedI64::from_num(-2),
            FixedI64::ZERO,
        ),
        utils::position(0.0, 1.0)
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
    let desired = utils::position(4.4, 5.0);
    assert_eq!(
        terrain::slide_toward(
            &grid,
            LayerMask::from(utils::GROUND),
            utils::position(4.7, 5.0),
            CellSize::ONE,
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
            LayerMask::from(utils::GROUND),
            utils::position(4.7, 5.0),
            CellSize::ONE,
            utils::position(4.9, 5.3),
            FixedU64::from_num(0.5),
        ),
        utils::position(4.9, 5.0)
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// An 8×8 single-layer grid with the given cells statically walled.
fn walled_grid(walls: &[(u32, u32)]) -> NavGrid {
    let mut grid = NavGrid::new(8, 8);
    grid.add_layer(utils::GROUND);
    for &(x, y) in walls {
        grid.set_occupied_by(utils::GROUND, CellPos::new(x, y), true);
    }
    grid
}
