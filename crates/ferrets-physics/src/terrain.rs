//! Bodies against standing terrain: what a circle may overlap, and how a
//! displacement commits without entering what it may not.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, location::Location};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::{mover_shape::MoverShape, nav_grid::NavGrid};

use crate::body;

/// Whether a body of `shape` at `position` lies wholly on the map and
/// overlaps only cells statically clear for it and joined, for its cliff
/// crossing, to the cell under its position — the cell whose level is the
/// body's.
pub fn body_fits(
    grid: &NavGrid,
    shape: MoverShape,
    position: FixedUVec2,
    radius: FixedU64,
) -> bool {
    let under = CellPos::from(position);
    on_map(grid, shape, position)
        && body::overlapped_cells(position, radius)
            .into_iter()
            .all(|cell| stands_on(grid, shape, under, cell))
}

/// Commits a desired position against the map's edges, the static plane and the
/// levels the body may cross: the desired point when the body lies wholly on
/// the map there, fits there and the cell under it is joined to the one it
/// leaves, else the desired point with one axis dropped — sliding along
/// whatever blocked it — else the position unmoved. A caller seeing no movement
/// out of a wanted one knows the body is walled off.
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
    shape: MoverShape,
    position: FixedUVec2,
    desired: FixedUVec2,
    radius: FixedU64,
) -> FixedUVec2 {
    let from = CellPos::from(position);
    let clipped: Vec<CellPos> = body::overlapped_cells(position, radius)
        .into_iter()
        .filter(|&cell| !stands_on(grid, shape, from, cell))
        .collect();
    let fits = |candidate: FixedUVec2| {
        let under = CellPos::from(candidate);
        on_map(grid, shape, candidate)
            && grid.joins(from, under, shape)
            && body::overlapped_cells(candidate, radius)
                .into_iter()
                .all(|cell| stands_on(grid, shape, under, cell) || clipped.contains(&cell))
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

/// The middle of a body at `location` displaced by a signed push per axis,
/// held so the body stays wholly on a map `map_size` cells across.
pub fn displaced(
    location: Location,
    push_x: FixedI64,
    push_y: FixedI64,
    map_size: CellSize,
) -> FixedUVec2 {
    let position = location.middle();
    let apply = |value: FixedU64, push: FixedI64| {
        if push >= FixedI64::ZERO {
            value + push.to_num::<FixedU64>()
        } else {
            value.saturating_sub((-push).to_num::<FixedU64>())
        }
    };
    Location::clamped(
        FixedUVec2::new(apply(position.x, push_x), apply(position.y, push_y)),
        location.size(),
        map_size,
    )
    .middle()
}

/// Whether a body of `shape` with its middle at `position` lies wholly on the
/// map `grid` covers.
fn on_map(grid: &NavGrid, shape: MoverShape, position: FixedUVec2) -> bool {
    Location::checked(position, shape.size, grid.size()).is_some()
}

/// Whether a body of `shape` over `under` may overlap `cell`: the cell is
/// statically clear for it and joined to `under` for its cliff crossing.
fn stands_on(grid: &NavGrid, shape: MoverShape, under: CellPos, cell: CellPos) -> bool {
    grid.cell_fits_statically(cell, shape) && grid.joins(under, cell, shape)
}
