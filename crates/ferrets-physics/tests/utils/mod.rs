#![allow(dead_code)]

use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};
use ferrets_physics::body::Body;

/// The layer every test body occupies unless a test says otherwise.
pub const GROUND: LayerId = LayerId::new(1);
/// A layer disjoint from [`GROUND`], for bodies that pass each other.
pub const AIR: LayerId = LayerId::new(2);

/// A cell coordinate written as decimal digits. Fixed-point throughout: the
/// value is the one the digits name, with no float in the way of it.
pub fn cells(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a length in cells"))
}

/// The same, where the value can point backwards.
pub fn signed_cells(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is an offset in cells"))
}

pub fn position(x: &str, y: &str) -> FixedUVec2 {
    FixedUVec2::new(cells(x), cells(y))
}

pub fn heading(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(signed_cells(x), signed_cells(y))
}

/// A half-cell-radius body at rest, filling a single cell, of the baseline
/// weight every test body carries unless it says otherwise.
pub fn resting(x: &str, y: &str) -> Body {
    Body {
        position: position(x, y),
        size: CellSize::ONE,
        radius: cells("0.5"),
        weight: FixedU64::ONE,
        mask: LayerMask::from(GROUND),
        heading: None,
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
        size: CellSize::new(size, size),
        radius: FixedU64::from_num(size) / 2,
        ..resting(x, y)
    }
}

/// A half-cell-radius body walking toward the given heading.
pub fn walking(x: &str, y: &str, toward_x: &str, toward_y: &str) -> Body {
    Body {
        heading: Some(heading(toward_x, toward_y)),
        ..resting(x, y)
    }
}

/// A half-cell-radius body at rest on the [`AIR`] layer.
pub fn flying(x: &str, y: &str) -> Body {
    Body {
        mask: LayerMask::from(AIR),
        ..resting(x, y)
    }
}

/// Every push is deterministic fixed-point math, so the expectations are
/// exact values, not signs — a changed magnitude is a changed contract.
pub fn push(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(signed_cells(x), signed_cells(y))
}
