mod utils;

use ferrets_math::{
    FixedI64, FixedU64,
    facing::{self, Facing},
    fixed_vec2::FixedVec2,
};

//
// ─── Bearings from a delta ────────────────────────────────────────────────────
//

#[test]
fn cardinal_deltas_land_exactly_on_cardinal_bearings() {
    assert_eq!(Facing::of(utils::vec2(0, -1)), Some(Facing::NORTH));
    assert_eq!(Facing::of(utils::vec2(1, 0)), Some(Facing::EAST));
    assert_eq!(Facing::of(utils::vec2(0, 1)), Some(Facing::SOUTH));
    assert_eq!(Facing::of(utils::vec2(-1, 0)), Some(Facing::WEST));
}

#[test]
fn diagonal_deltas_land_exactly_between_cardinal_bearings() {
    let eighth = (facing::PER_TURN / 8) as u16;

    assert_eq!(bearing(1, -1), eighth);
    assert_eq!(bearing(1, 1), eighth * 3);
    assert_eq!(bearing(-1, 1), eighth * 5);
    assert_eq!(bearing(-1, -1), eighth * 7);
}

#[test]
fn bearing_reads_direction_not_length() {
    assert_eq!(
        Facing::of(utils::vec2(3, -3)),
        Facing::of(utils::vec2(1, -1))
    );
    assert_eq!(
        Facing::of(FixedVec2::new(
            FixedI64::lit("0.01"),
            FixedI64::lit("-0.01")
        )),
        Facing::of(utils::vec2(1, -1))
    );
}

#[test]
fn delta_pointing_nowhere_has_no_bearing() {
    assert_eq!(Facing::of(FixedVec2::ZERO), None);
}

#[test]
fn bearings_grow_clockwise_through_quarter() {
    let mut previous = 0;
    // A quarter's worth of directions from due north round to due east, by the
    // ratio of the two axes rather than by any angle the bearing itself supplies.
    for across in 0..=10u32 {
        let delta = FixedVec2::new(FixedI64::from_num(across), FixedI64::from_num(-10));
        let bits = Facing::of(delta)
            .expect("a delta with length has a bearing")
            .to_bits();
        assert!(
            bits >= previous,
            "bearing fell back at {across}/10: {bits} after {previous}"
        );
        previous = bits;
    }
    assert_eq!(previous, (facing::PER_TURN / 8) as u16);
}

#[test]
fn approximated_bearings_stay_within_tenth_of_degree() {
    // tan 30° and tan 60°, the worst that a polynomial exact at 0° and 45° can do.
    let thirty = Facing::of(FixedVec2::new(FixedI64::lit("0.57735"), -FixedI64::ONE));
    let sixty = Facing::of(FixedVec2::new(FixedI64::ONE, -FixedI64::lit("0.57735")));
    let tolerance = facing::units_of_degrees(FixedU64::lit("0.1"));

    assert!(
        thirty.unwrap().distance(exactly(30)) <= tolerance,
        "30° off by {}",
        thirty.unwrap().distance(exactly(30))
    );
    assert!(
        sixty.unwrap().distance(exactly(60)) <= tolerance,
        "60° off by {}",
        sixty.unwrap().distance(exactly(60))
    );
}

//
// ─── Differences ─────────────────────────────────────────────────────────────
//

#[test]
fn difference_is_signed_clockwise() {
    assert_eq!(
        Facing::NORTH.difference(Facing::EAST),
        (facing::PER_TURN / 4) as i32
    );
    assert_eq!(
        Facing::EAST.difference(Facing::NORTH),
        -((facing::PER_TURN / 4) as i32)
    );
}

#[test]
fn difference_takes_short_way_across_north() {
    let east_of_north = Facing::from_bits(64);
    let west_of_north = Facing::from_bits(u16::MAX - 63);

    assert_eq!(east_of_north.difference(west_of_north), -128);
    assert_eq!(west_of_north.difference(east_of_north), 128);
}

#[test]
fn difference_never_exceeds_half_turn() {
    let half = facing::PER_TURN / 2;

    for bits in [0u16, 1, 12_345, 32_767, 32_768, 60_000, u16::MAX] {
        assert!(Facing::NORTH.distance(Facing::from_bits(bits)) <= half);
    }
}

//
// ─── Turning ─────────────────────────────────────────────────────────────────
//

#[test]
fn turn_within_reach_is_taken_up_whole() {
    let quarter = facing::PER_TURN / 4;

    assert_eq!(
        Facing::NORTH.turn_toward(Facing::EAST, quarter),
        Facing::EAST
    );
    assert_eq!(
        Facing::NORTH.turn_toward(Facing::EAST, quarter + 1),
        Facing::EAST
    );
}

#[test]
fn turn_beyond_reach_moves_by_allowance() {
    let quarter = facing::PER_TURN / 4;

    assert_eq!(
        Facing::NORTH.turn_toward(Facing::EAST, 100),
        Facing::from_bits(100)
    );
    assert_eq!(
        Facing::EAST.turn_toward(Facing::NORTH, 100),
        Facing::from_bits(quarter as u16 - 100)
    );
}

#[test]
fn turn_takes_short_way_and_wraps_past_north() {
    let west_of_north = Facing::from_bits(u16::MAX - 99);

    assert_eq!(
        west_of_north.turn_toward(Facing::NORTH, 50),
        Facing::from_bits(u16::MAX - 49)
    );
    assert_eq!(west_of_north.turn_toward(Facing::NORTH, 100), Facing::NORTH);
    assert_eq!(
        Facing::from_bits(50).turn_toward(west_of_north, 100),
        Facing::from_bits(u16::MAX - 49)
    );
}

#[test]
fn turn_across_half_circle_completes_with_half_turn_allowance() {
    // Dead opposite is the one turn a clamp one unit low would leave forever
    // short: half a circle away in either direction, and never more.
    let half = facing::PER_TURN / 2;

    assert_eq!(
        Facing::SOUTH.turn_toward(Facing::NORTH, half),
        Facing::NORTH
    );
    assert_eq!(
        Facing::SOUTH.turn_toward(Facing::NORTH, u32::MAX),
        Facing::NORTH
    );
}

#[test]
fn turn_of_nothing_stays_put() {
    assert_eq!(Facing::EAST.turn_toward(Facing::WEST, 0), Facing::EAST);
}

#[test]
fn turn_toward_own_bearing_stays_put() {
    assert_eq!(Facing::EAST.turn_toward(Facing::EAST, 100), Facing::EAST);
}

//
// ─── Degrees ─────────────────────────────────────────────────────────────────
//

#[test]
fn whole_turn_of_degrees_is_whole_turn_of_units() {
    assert_eq!(
        facing::units_of_degrees(utils::uscalar(360)),
        facing::PER_TURN
    );
    assert_eq!(
        facing::units_of_degrees(utils::uscalar(90)),
        facing::PER_TURN / 4
    );
    assert_eq!(facing::units_of_degrees(utils::uscalar(0)), 0);
}

#[test]
fn fractional_degrees_convert() {
    assert_eq!(
        facing::units_of_degrees(FixedU64::lit("22.5")),
        facing::PER_TURN / 16
    );
    assert_eq!(
        facing::units_of_degrees(FixedU64::lit("0.5")),
        facing::PER_TURN / 720
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The bearing of a whole-number delta, in units.
fn bearing(x: i32, y: i32) -> u16 {
    Facing::of(utils::vec2(x, y))
        .expect("a delta with length has a bearing")
        .to_bits()
}

/// The bearing `degrees` clockwise from north, exactly.
fn exactly(degrees: u32) -> Facing {
    Facing::from_bits(facing::units_of_degrees(utils::uscalar(degrees)) as u16)
}
