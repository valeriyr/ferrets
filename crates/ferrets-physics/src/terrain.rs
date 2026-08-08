//! Bodies against standing terrain: what a circle may overlap, and how a
//! displacement commits without entering what it may not.

use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{layer_mask::LayerMask, nav_grid::NavGrid};

use crate::body;

/// Whether a body anchored at `position` overlaps only statically passable
/// cells.
pub fn body_fits(grid: &NavGrid, mask: LayerMask, position: FixedUVec2, radius: FixedU64) -> bool {
    body::overlapped_cells(position, radius)
        .into_iter()
        .all(|cell| grid.is_statically_passable_by(mask, cell))
}

/// Commits a desired position against the static plane: the desired point
/// when the body fits there, else the desired point with one axis dropped —
/// sliding along whatever blocked it — else the position unmoved. A caller
/// seeing no movement out of a wanted one knows the body is walled off.
pub fn slide_toward(
    grid: &NavGrid,
    mask: LayerMask,
    position: FixedUVec2,
    desired: FixedUVec2,
    radius: FixedU64,
) -> FixedUVec2 {
    [
        desired,
        FixedUVec2::new(desired.x, position.y),
        FixedUVec2::new(position.x, desired.y),
    ]
    .into_iter()
    .find(|&candidate| body_fits(grid, mask, candidate, radius))
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
