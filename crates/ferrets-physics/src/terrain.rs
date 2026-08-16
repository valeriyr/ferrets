//! Bodies against standing terrain: what a circle may overlap, and how a
//! displacement commits without entering what it may not.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::NavGrid};

use crate::body;

/// Whether a body of `size` anchored at `position` overlaps only statically
/// passable cells.
pub fn body_fits(
    grid: &NavGrid,
    mask: LayerMask,
    position: FixedUVec2,
    size: CellSize,
    radius: FixedU64,
) -> bool {
    body::overlapped_cells(position, size, radius)
        .into_iter()
        .all(|cell| grid.is_statically_passable_by(mask, cell))
}

/// Commits a desired position against the static plane: the desired point
/// when the body fits there, else the desired point with one axis dropped —
/// sliding along whatever blocked it — else the position unmoved. A caller
/// seeing no movement out of a wanted one knows the body is walled off.
///
/// The kept axis is the step's dominant one: a walk skimming a wall wants to
/// keep its along-wall progress, not be diverted down its own faint sideways
/// component — that diversion is how a body rounding a corner slid off along
/// the wrong face. Ties keep the x axis, deterministically.
///
/// A body that already clips blocked ground — a building raised against its
/// edge — is not frozen by that clip: a step may keep the blocked cells the
/// body already overlaps, it just may not overlap a new one. The clip never
/// spreads to fresh cells, so the body walks itself free; within the cells
/// it already clips it moves freely, which is accepted over freezing.
pub fn slide_toward(
    grid: &NavGrid,
    mask: LayerMask,
    position: FixedUVec2,
    size: CellSize,
    desired: FixedUVec2,
    radius: FixedU64,
) -> FixedUVec2 {
    let clipped: Vec<CellPos> = body::overlapped_cells(position, size, radius)
        .into_iter()
        .filter(|&cell| !grid.is_statically_passable_by(mask, cell))
        .collect();
    let fits = |candidate: FixedUVec2| {
        body::overlapped_cells(candidate, size, radius)
            .into_iter()
            .all(|cell| grid.is_statically_passable_by(mask, cell) || clipped.contains(&cell))
    };

    let along_x = FixedUVec2::new(desired.x, position.y);
    let along_y = FixedUVec2::new(position.x, desired.y);
    let candidates = if desired.x.abs_diff(position.x) >= desired.y.abs_diff(position.y) {
        [desired, along_x, along_y]
    } else {
        [desired, along_y, along_x]
    };
    candidates
        .into_iter()
        .find(|&candidate| fits(candidate))
        .unwrap_or(position)
}

/// `position` displaced by a signed push per axis, clamped at the map's
/// origin.
pub fn displaced(position: FixedUVec2, push_x: FixedI64, push_y: FixedI64) -> FixedUVec2 {
    let apply = |value: FixedU64, push: FixedI64| {
        if push >= FixedI64::ZERO {
            value + push.to_num::<FixedU64>()
        } else {
            value.saturating_sub((-push).to_num::<FixedU64>())
        }
    };
    FixedUVec2::new(apply(position.x, push_x), apply(position.y, push_y))
}
