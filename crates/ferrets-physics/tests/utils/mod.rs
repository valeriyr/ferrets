#![allow(dead_code)]

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, location::Location};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_nav_grid::{
    layer_id::LayerId, mover_shape::MoverShape, nav_grid::NavGrid, terrain_id::TerrainId,
    terrain_mask::TerrainMask,
};
use ferrets_physics::{
    body::{Bearing, Body, Heading},
    contact::Meeting,
    resolve,
};

/// The layer every test body occupies unless a test says otherwise.
pub const GROUND: LayerId = LayerId::new(1);
/// A layer disjoint from [`GROUND`], for bodies that pass each other.
pub const AIR: LayerId = LayerId::new(2);
/// The map every test body stands on unless a test says otherwise.
pub const MAP_SIZE: CellSize = CellSize::new(64, 64);
/// The terrain every test grid is made of.
pub const LAND: TerrainId = TerrainId::new(1);
/// A second terrain, for cells a land body may not overlap.
pub const WATER: TerrainId = TerrainId::new(2);

/// A one-cell body over land on [`GROUND`].
pub fn ground_shape() -> MoverShape {
    MoverShape::point(TerrainMask::from(LAND), GROUND, None)
}

/// A cell coordinate written as decimal digits. Fixed-point throughout: the
/// value is the one the digits name, with no float in the way of it.
pub fn cells(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a length in cells"))
}

/// The same, where the value can point backwards.
pub fn signed_cells(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is an offset in cells"))
}

/// A point on the map, written as decimals.
pub fn point(x: &str, y: &str) -> FixedUVec2 {
    FixedUVec2::new(cells(x), cells(y))
}

/// A point on the map given by its coordinates' raw bits, for a value no
/// short decimal names.
pub fn point_bits(x: u64, y: u64) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_bits(x), FixedU64::from_bits(y))
}

/// The place of a body of `size` with its middle at `(x, y)` on [`MAP_SIZE`].
pub fn location(x: &str, y: &str, size: CellSize) -> Location {
    Location::new(point(x, y), size, MAP_SIZE)
}

/// A walker's heading toward the offset written as decimals.
pub fn heading(x: &str, y: &str) -> Heading {
    Heading::Toward(FixedVec2::new(signed_cells(x), signed_cells(y)))
}

/// A half-cell-radius body standing idle with its middle at `(x, y)`,
/// filling a single cell, of the baseline weight every test body carries
/// unless it says otherwise.
pub fn resting(x: &str, y: &str) -> Body {
    Body {
        location: location(x, y, CellSize::ONE),
        radius: cells("0.5"),
        weight: FixedU64::ONE,
        mover: ground_shape(),
        bearing: Bearing::Idle,
    }
}

/// A half-cell-radius working body with its middle at `(x, y)`.
pub fn working(x: &str, y: &str) -> Body {
    Body {
        bearing: Bearing::Working,
        ..resting(x, y)
    }
}

/// A half-cell-radius inert body with its middle at `(x, y)`.
pub fn inert(x: &str, y: &str) -> Body {
    Body {
        bearing: Bearing::Inert,
        ..resting(x, y)
    }
}

/// A half-cell-radius planted body with its middle at `(x, y)`.
pub fn planted(x: &str, y: &str) -> Body {
    Body {
        bearing: Bearing::Planted,
        ..resting(x, y)
    }
}

/// A half-cell-radius body unable to move, with its middle at `(x, y)`.
pub fn fixed(x: &str, y: &str) -> Body {
    Body {
        bearing: Bearing::Fixed,
        ..resting(x, y)
    }
}

/// The same body, weighing `weight` against whatever it meets.
pub fn weighing(body: Body, weight: &str) -> Body {
    Body {
        weight: cells(weight),
        ..body
    }
}

/// A body filling a `size`-by-`size` footprint, its circle inscribed in it.
pub fn wide(x: &str, y: &str, size: u32) -> Body {
    Body {
        location: location(x, y, CellSize::new(size, size)),
        radius: FixedU64::from_num(size) / 2,
        mover: MoverShape::new(
            TerrainMask::from(LAND),
            GROUND,
            GROUND,
            CellSize::new(size, size),
            None,
        ),
        ..resting(x, y)
    }
}

/// A half-cell-radius body walking toward the given heading.
pub fn walking(x: &str, y: &str, toward_x: &str, toward_y: &str) -> Body {
    Body {
        bearing: Bearing::Walking(heading(toward_x, toward_y)),
        ..resting(x, y)
    }
}

/// A half-cell-radius body at rest on the [`AIR`] layer.
pub fn flying(x: &str, y: &str) -> Body {
    Body {
        mover: MoverShape::point(TerrainMask::from(LAND), AIR, None),
        ..resting(x, y)
    }
}

/// Every push is deterministic fixed-point math, so the expectations are
/// exact values, not signs — a changed magnitude is a changed contract.
pub fn push(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(signed_cells(x), signed_cells(y))
}

/// An open [`MAP_SIZE`] grid of land, flat on [`GROUND`] and [`AIR`].
pub fn open_grid() -> NavGrid {
    let mut grid = NavGrid::new(MAP_SIZE.width, MAP_SIZE.height, |_| LAND);
    grid.add_flat_layer(GROUND, 1);
    grid.add_flat_layer(AIR, 1);
    grid
}

/// How far one contact pass over [`open_grid`] moves each of `bodies`, the
/// pairs meeting as `meeting` says.
pub fn pushes(bodies: &[Body], meeting: impl Fn(usize, usize) -> Meeting) -> Vec<FixedVec2> {
    pushes_over(&open_grid(), bodies, meeting)
}

/// How far one contact pass over `grid` moves each of `bodies`.
pub fn pushes_over(
    grid: &NavGrid,
    bodies: &[Body],
    meeting: impl Fn(usize, usize) -> Meeting,
) -> Vec<FixedVec2> {
    resolve::settle(bodies, meeting, grid)
        .into_iter()
        .zip(bodies)
        .map(|(location, body)| {
            let (to, from) = (location.middle(), body.location.middle());
            FixedVec2::new(
                to.x.to_num::<FixedI64>() - from.x.to_num::<FixedI64>(),
                to.y.to_num::<FixedI64>() - from.y.to_num::<FixedI64>(),
            )
        })
        .collect()
}

/// An open grid with rows 4 and 6 walled from column 0 to 12: a corridor one
/// cell high along row 5.
pub fn corridor_grid() -> NavGrid {
    let mut grid = open_grid();
    for x in 0..13 {
        grid.set_occupied(GROUND, CellPos::new(x, 4), true);
        grid.set_occupied(GROUND, CellPos::new(x, 6), true);
    }
    grid
}
