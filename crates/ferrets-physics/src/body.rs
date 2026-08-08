//! A moving thing's physical presence: a circle over the cell grid.

use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::layer_mask::LayerMask;

/// One body during a contact-resolution pass: a circle centered half a cell
/// past its anchor position — where it is rendered — with a radius in cells.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    /// The anchor position, in cells with sub-cell precision.
    pub position: FixedUVec2,
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

/// The cell under a body's center — the cell the body visually stands on,
/// and the one cell a settled body comes to hold exclusively (see
/// [`contact::separations`](crate::contact::separations)).
pub fn center_cell(position: FixedUVec2) -> CellPos {
    let half = FixedU64::from_num(0.5);
    CellPos::from(FixedUVec2::new(position.x + half, position.y + half))
}

/// The cells a body anchored at `position` physically overlaps, in
/// row-major order: a bounding-box corner cell the circle misses does not
/// count, and touching a cell boundary exactly does not enter the next
/// cell. Coordinates are unclamped — a body hanging over the grid's far
/// edge names cells past it, which readers treat as they see fit (the
/// terrain checks read them as impassable).
pub fn overlapped_cells(position: FixedUVec2, radius: FixedU64) -> Vec<CellPos> {
    let half = FixedU64::from_num(0.5);
    let center = FixedUVec2::new(position.x + half, position.y + half);
    let cells = |anchor: FixedU64| {
        let low = (anchor + half).saturating_sub(radius);
        let high = anchor + half + radius;
        let first = low.floor().to_num::<u32>();
        let last = if high.frac() == FixedU64::ZERO {
            high.to_num::<u32>().saturating_sub(1)
        } else {
            high.floor().to_num::<u32>()
        };
        first..=last
    };
    let mut overlapped = Vec::new();
    for y in cells(position.y) {
        for x in cells(position.x) {
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
