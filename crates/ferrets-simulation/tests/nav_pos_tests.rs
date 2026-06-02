mod utils;

use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::pathfinding::nav_pos::NavPos;

//
// ─── From world ───────────────────────────────────────────────────────────────
//

#[test]
fn from_world_whole_numbers() {
    assert_eq!(NavPos::from(utils::world(3, 7)), utils::nav(3, 7));
}

#[test]
fn from_world_floors_fractional() {
    let p = FixedUVec2::new(FixedU64::from_num(1.7_f32), FixedU64::from_num(2.3_f32));
    assert_eq!(NavPos::from(p), utils::nav(1, 2));
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
    assert_eq!(FixedUVec2::from(NavPos::from(original)), original);
}
