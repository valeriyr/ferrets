//! `CellPos` grid coordinate: world conversions, ordering, and rectangle
//! clamping.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};

//
// ─── Default ──────────────────────────────────────────────────────────────────
//

#[test]
fn default_is_origin() {
    assert_eq!(CellPos::default(), CellPos::new(0, 0));
}

//
// ─── Ordering ─────────────────────────────────────────────────────────────────
//

#[test]
fn ordering_is_lexicographic_x_then_y() {
    // Deterministic tie-breaks and ordered containers lean on this order:
    // x decides first, y only between equals.
    assert!(utils::nav(1, 9) < utils::nav(2, 0));
    assert!(utils::nav(3, 2) < utils::nav(3, 5));
    assert_eq!(
        [utils::nav(2, 0), utils::nav(1, 9), utils::nav(1, 2)]
            .into_iter()
            .min(),
        Some(utils::nav(1, 2))
    );
}

//
// ─── From world ───────────────────────────────────────────────────────────────
//

#[test]
fn from_world_whole_numbers() {
    assert_eq!(CellPos::from(utils::world(3, 7)), utils::nav(3, 7));
}

#[test]
fn from_world_floors_fractional() {
    let p = FixedUVec2::new(FixedU64::from_num(1.7_f32), FixedU64::from_num(2.3_f32));
    assert_eq!(CellPos::from(p), utils::nav(1, 2));
}

//
// ─── To world ─────────────────────────────────────────────────────────────────
//

#[test]
fn to_world_gives_origin_corner() {
    assert_eq!(FixedUVec2::from(utils::nav(4, 5)), utils::world(4, 5));
}

#[test]
fn round_trip_is_lossless_for_whole_numbers() {
    let original = utils::world(6, 9);
    assert_eq!(FixedUVec2::from(CellPos::from(original)), original);
}

//
// ─── clamp_to_rect ────────────────────────────────────────────────────────────
//
// Tests use a 2×2 rectangle at origin (3,3): cells (3,3) (4,3) (3,4) (4,4).
//

#[test]
fn clamp_to_rect_returns_self_when_inside() {
    for cell in [(3, 3), (4, 3), (3, 4), (4, 4)] {
        let p = utils::nav(cell.0, cell.1);
        assert_eq!(p.clamp_to_rect(RECT), p);
    }
}

#[test]
fn clamp_to_rect_clamps_each_axis_independently() {
    // Far to the lower-right: both axes clamp to the near corner (4,4).
    assert_eq!(utils::nav(9, 7).clamp_to_rect(RECT), utils::nav(4, 4));
    // Left of and level with the rect: x clamps up to 3, y stays at 4.
    assert_eq!(utils::nav(0, 4).clamp_to_rect(RECT), utils::nav(3, 4));
    // Above the right edge: x stays at 4, y clamps up to 3.
    assert_eq!(utils::nav(4, 0).clamp_to_rect(RECT), utils::nav(4, 3));
}

#[test]
fn clamp_to_rect_handles_single_cell_footprint() {
    let cell = utils::nav(5, 5);
    // A 1×1 rect is a single cell — every point clamps onto it.
    assert_eq!(utils::nav(0, 0).clamp_to_rect(CellRect::cell(cell)), cell);
    assert_eq!(cell.clamp_to_rect(CellRect::cell(cell)), cell);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The 2×2 rectangle at origin (3,3) the clamping tests use.
const RECT: CellRect = CellRect::new(CellPos::new(3, 3), CellSize::new(2, 2));
