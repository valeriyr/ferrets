#![allow(dead_code)]

use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{layer_id::LayerId, layer_mask::LayerMask};
use ferrets_physics::body::Body;

/// The layer every test body occupies unless a test says otherwise.
pub const GROUND: LayerId = LayerId::new(1);
/// A layer disjoint from [`GROUND`], for bodies that pass each other.
pub const AIR: LayerId = LayerId::new(2);

pub fn position(x: f64, y: f64) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

pub fn heading(x: f64, y: f64) -> FixedVec2 {
    FixedVec2::new(FixedI64::from_num(x), FixedI64::from_num(y))
}

/// A half-cell-radius body at rest, filling a single cell.
pub fn resting(x: f64, y: f64) -> Body {
    Body {
        position: position(x, y),
        size: CellSize::ONE,
        radius: FixedU64::from_num(0.5),
        mask: LayerMask::from(GROUND),
        heading: None,
    }
}

/// A body filling a `size`-by-`size` footprint, its circle inscribed in it.
pub fn wide(x: f64, y: f64, size: u32) -> Body {
    Body {
        size: CellSize::new(size, size),
        radius: FixedU64::from_num(size) / 2,
        ..resting(x, y)
    }
}

/// A half-cell-radius body walking toward the given heading.
pub fn walking(x: f64, y: f64, toward_x: f64, toward_y: f64) -> Body {
    Body {
        heading: Some(heading(toward_x, toward_y)),
        ..resting(x, y)
    }
}

/// A half-cell-radius body at rest on the [`AIR`] layer.
pub fn flying(x: f64, y: f64) -> Body {
    Body {
        mask: LayerMask::from(AIR),
        ..resting(x, y)
    }
}

/// Every push is deterministic fixed-point math, so the expectations are
/// exact values, not signs — a changed magnitude is a changed contract.
pub fn push(x: f64, y: f64) -> FixedVec2 {
    FixedVec2::new(FixedI64::from_num(x), FixedI64::from_num(y))
}
