//! A moving thing's physical presence: a circle over the cell grid.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::layer_mask::LayerMask;

/// One body during a contact-resolution pass: a circle inscribed in the
/// footprint anchored at its position — where it is rendered — with a radius in
/// cells.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    /// The anchor position, in cells with sub-cell precision. The footprint
    /// extends from here, as a standing entity's does.
    pub position: FixedUVec2,
    /// The size of the footprint the body moves as, which places its circle:
    /// the circle is centered half the size past the anchor.
    pub size: CellSize,
    /// The circle's radius, in cells.
    pub radius: FixedU64,
    /// The layers the body occupies; bodies with disjoint masks pass
    /// through each other — including an empty mask, which touches nothing
    /// at all.
    pub mask: LayerMask,
    /// Where the body is walking this tick — the offset toward its current
    /// waypoint — or `None` at rest. Contacts consult it for the swerving
    /// traffic rule.
    pub heading: Option<FixedVec2>,
}

/// The center of the circle a footprint of `size` anchored at `position`
/// inscribes — half the size past the anchor, which is half a cell for the
/// single-cell case.
pub fn center(position: FixedUVec2, size: CellSize) -> FixedUVec2 {
    let half = |extent: u32| FixedU64::from_num(extent) / 2;
    FixedUVec2::new(
        position.x + half(size.width),
        position.y + half(size.height),
    )
}

/// The anchor of the footprint a body at `position` currently occupies:
/// the position rounded to the nearest lattice point, which is the footprint
/// the eye puts it on and the one a settled body comes to hold exclusively
/// (see [`contact::separations`](crate::contact::separations)).
///
/// Independent of footprint size, because the footprint extends from the
/// anchor either way — a wider body holds more cells from the same anchor.
pub fn anchor(position: FixedUVec2) -> CellPos {
    let half = FixedU64::from_num(0.5);
    CellPos::from(FixedUVec2::new(position.x + half, position.y + half))
}

/// The cells a body of `size` anchored at `position` physically overlaps, in
/// row-major order: a bounding-box corner cell the circle misses does not
/// count, and touching a cell boundary exactly does not enter the next
/// cell. Coordinates are unclamped — a body hanging over the grid's far
/// edge names cells past it, which readers treat as they see fit (the
/// terrain checks read them as impassable).
pub fn overlapped_cells(position: FixedUVec2, size: CellSize, radius: FixedU64) -> Vec<CellPos> {
    let center = center(position, size);
    let cells = |center: FixedU64| {
        let low = center.saturating_sub(radius);
        let high = center + radius;
        let first = low.floor().to_num::<u32>();
        let last = if high.frac() == FixedU64::ZERO {
            high.to_num::<u32>().saturating_sub(1)
        } else {
            high.floor().to_num::<u32>()
        };
        first..=last
    };
    let mut overlapped = Vec::new();
    for y in cells(center.y) {
        for x in cells(center.x) {
            // Exact on raw bits: overlap requires the circle to reach past
            // the cell's nearest point strictly.
            let nearest = |value: FixedU64, cell: u32| {
                value.clamp(FixedU64::from_num(cell), FixedU64::from_num(cell + 1))
            };
            let off_x = center.x.abs_diff(nearest(center.x, x)).to_bits() as u128;
            let off_y = center.y.abs_diff(nearest(center.y, y)).to_bits() as u128;
            let reach = radius.to_bits() as u128;
            if off_x * off_x + off_y * off_y < reach * reach {
                overlapped.push(CellPos::new(x, y));
            }
        }
    }
    overlapped
}
