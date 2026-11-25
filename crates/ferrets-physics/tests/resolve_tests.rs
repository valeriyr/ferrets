//! The contact pass: bodies never pushed toward what cannot make way.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, location::Location};
use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};
use ferrets_nav_grid::nav_grid::NavGrid;
use ferrets_physics::{
    body::{Bearing, Body, Heading},
    contact::Meeting,
};

//
// ─── Shut ways ────────────────────────────────────────────────────────────────
//

/// A body is never pushed toward what cannot make way, judged all along the
/// line: in a corridor one cell high, a walker 0.125 into an idle body that
/// touches another idle body that touches an idle body it meets unyielding
/// takes the whole 0.125 back, and nothing ahead of it moves.
#[test]
fn walker_into_shut_chain_takes_whole_overlap() {
    let pushes = utils::pushes_over(
        &utils::corridor_grid(),
        &[
            utils::walking("2", "5.5", "1", "0"),
            utils::resting("2.875", "5.5"),
            utils::resting("3.875", "5.5"),
            utils::resting("4.875", "5.5"),
        ],
        |_, second| unyielding_with(3, second),
    );

    // 2.875 − 2 = 0.875 apart, 0.125 inside the touching 0.5 + 0.5 = 1; its
    // clockwise lean is refused by the corridor's walls.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0"),
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// The terrain makes no way either: an idle body flush against a wall
/// leaves the walker 0.125 into it the whole overlap, and steps aside along
/// the wall.
#[test]
fn body_against_wall_leaves_walker_whole_overlap() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(6, 5), true);

    let pushes = utils::pushes_over(
        &grid,
        &[
            utils::walking("4.625", "5.5", "1", "0"),
            utils::resting("5.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // The idle body spans 5–6, its edge on the wall at x 6; the walker,
    // 0.875 from it, keeps its own quarter, 0.03125 (three times as stiff),
    // takes the idle body's 0.09375 too, 0.125 in all, and leans clockwise by
    // a quarter of what it took: 0.09375 / 4 = 0.0234375. The idle body,
    // pressed dead-on, moves nothing along the line and is nudged aside by a
    // quarter of its share, 0.0234375 toward −y, along the wall.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0.0234375"),
            utils::push("0", "-0.0234375")
        ]
    );
}

/// Nor does the map's edge: an idle body flush against the origin leaves
/// the walker the whole overlap, and steps aside along the edge.
#[test]
fn body_against_map_edge_leaves_walker_whole_overlap() {
    let pushes = utils::pushes(
        &[
            utils::walking("1.375", "5.5", "-1", "0"),
            utils::resting("0.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 0.125 back, leaning clockwise of its way out (+x) by a quarter of the
    // idle body's 0.09375 it took: 0.0234375 toward −y. The idle body is
    // nudged aside by a quarter of its share, clockwise of its way out (−x):
    // 0.0234375 toward +y.
    assert_eq!(
        pushes,
        vec![
            utils::push("0.125", "-0.0234375"),
            utils::push("0", "0.0234375")
        ]
    );
}

/// Nor does a planted body: in a corridor, an idle body touching a planted
/// one leaves the walker the whole overlap.
#[test]
fn body_against_planted_body_leaves_walker_whole_overlap() {
    let pushes = utils::pushes_over(
        &utils::corridor_grid(),
        &[
            utils::walking("3", "5.5", "1", "0"),
            utils::resting("3.875", "5.5"),
            utils::planted("4.875", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 3.875 − 3 = 0.875 apart, 0.125 inside the touching 1; its clockwise
    // lean is refused by the corridor's walls.
    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), FixedVec2::ZERO, FixedVec2::ZERO]
    );
}

/// A body pressed dead into one that will not give way, with room around it,
/// makes no way along the line: in open ground an idle body touching a body
/// it meets unyielding moves only aside, and the walker takes back what it
/// cannot move.
#[test]
fn body_sliding_around_unyielding_body_leaves_walker_whole_overlap() {
    let pushes = utils::pushes(
        &[
            utils::walking("2", "5.5", "1", "0"),
            utils::resting("2.875", "5.5"),
            utils::resting("3.875", "5.5"),
        ],
        |_, second| unyielding_with(2, second),
    );

    // The walker presses, three times as stiff: a quarter of 0.125 back,
    // 0.03125. The idle body's three quarters, 0.09375, makes none of its way
    // along the line, so the walker takes that too, 0.125 in all, leaning
    // clockwise by a quarter of what it took, 0.0234375.
    assert_eq!(pushes[0], utils::push("-0.125", "0.0234375"));
    // The idle body makes nothing along the line: the ground is the tighter
    // limit, so it is pushed its whole 0.09375, nudged a quarter of that,
    // 0.0234375, toward −y (pressed dead-on); the push meets the unyielding
    // body at once and slides along its rim by a quarter of its length,
    // √(0.09375² + 0.0234375²) / 4 ≈ 0.0242, toward −y.
    assert_eq!(pushes[1], utils::push("0", "-0.024158822"));
    assert_eq!(pushes[2], FixedVec2::ZERO);
}

/// A planted body never gives way to a body squeezed against it: planted P
/// overlapping idle B by 0.125, B overlapping by 0.125 an idle body it meets
/// unyielding — B makes nothing along either line and slides out aside, P
/// stays, and the third body takes only its own half of its overlap with B,
/// 0.0625.
#[test]
fn planted_body_never_gives_way_to_squeezed_one() {
    let pushes = utils::pushes(
        &[
            utils::planted("5.625", "5.5"),
            utils::resting("6.5", "5.5"),
            utils::resting("7.375", "5.5"),
        ],
        |_, second| unyielding_with(2, second),
    );

    // B takes P's whole 0.125 east and its own half of its overlap with the
    // third body, 0.0625, west: 0.0625 east, into the third body, which it
    // already overlaps, met at once and slid along its rim by a quarter,
    // 0.015625.
    assert_eq!(
        pushes,
        vec![
            FixedVec2::ZERO,
            utils::push("0", "0.015625"),
            utils::push("0.0625", "0")
        ]
    );
}

/// Two idle bodies meeting unyielding halve an overlap, neither shut toward
/// the other.
#[test]
fn idle_unyielding_pair_halves_overlap() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::resting("5.75", "5.5")],
        |_, _| Meeting::Unyielding,
    );

    // 1 − 0.75 = 0.25, 0.125 each.
    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.125", "0")]
    );
}

/// A walker taking the whole overlap from a shut body leans past it the
/// way it heads, by the part of its heading across the line, at least a
/// quarter: heading up and to the right (−y) at 45° into an idle body in a
/// notch walled on three sides, it is pushed back 0.125, and leans by what it
/// took, the idle body's 0.09375, times sin 45°: 0.09375 × √2 / 2 ≈
/// 0.0662912605, toward −y, counterclockwise of its way out.
#[test]
fn walker_leans_past_shut_body_way_it_heads() {
    let mut grid = utils::open_grid();
    for (x, y) in [(5, 5), (4, 4), (4, 6)] {
        grid.set_occupied(utils::GROUND, CellPos::new(x, y), true);
    }
    let pushes = utils::pushes_over(
        &grid,
        &[
            utils::walking("3.625", "5.5", "1", "-1"),
            utils::resting("4.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "-0.0662912605"), FixedVec2::ZERO]
    );
}

/// What a body cannot move along the line goes back to its walker in part
/// too: an idle body touching, at an angle, an idle body it meets unyielding
/// — that one 0.6 along and 0.8 across the walker's line from it — makes
/// 0.625 of a step along the line, sliding along the other's rim, so the
/// walker takes back the rest of the idle body's share as well as its own.
#[test]
fn body_leaning_on_unyielding_body_at_angle_hands_walker_what_it_cannot_move() {
    let pushes = utils::pushes(
        &[
            utils::walking("2", "5.5", "1", "0"),
            utils::resting("2.875", "5.5"),
            utils::resting("3.475", "6.3"),
        ],
        |_, second| unyielding_with(2, second),
    );

    // The walker keeps its quarter of 0.125, 0.03125. The idle body's sliver
    // toward the unyielding body slides along its rim, the tangent (−0.8, 0.6)
    // carrying 0.8 of it, 0.64 along the line. The walker takes back the idle
    // body's 0.09375 × 0.36 = 0.03375 too, 0.065 in all, and leans clockwise
    // by a quarter of what it took, 0.0084375.
    assert_eq!(pushes[0], utils::push("-0.065", "0.0084374999"));
    // The ground is the tighter limit, so the idle body is pushed its whole
    // 0.09375 along x, nudged a quarter of that, 0.0234375, toward −y
    // (pressed dead-on); the push slides along the other's rim by what it
    // carries along the tangent (0.8, −0.6): 0.09375 × 0.8 + 0.0234375 × 0.6
    // = 0.0890625, 0.07125 along x and 0.0534375 toward −y.
    assert_eq!(pushes[1], utils::push("0.0712500003", "-0.0534375003"));
    assert_eq!(pushes[2], FixedVec2::ZERO);
}

/// What a chain's front cannot move goes back down the whole chain: in open
/// ground a walker 0.125 into idle B, B touching idle C, C touching a
/// planted P dead-on — C would slide along P's rim but makes none of its way
/// along the line, so B makes none of its way toward C: B moves nothing
/// into C, only aside, and the walker takes back B's share too.
#[test]
fn walker_into_chain_whose_front_only_slides_takes_whole_overlap() {
    let pushes = utils::pushes(
        &[
            utils::walking("2", "5.5", "1", "0"),
            utils::resting("2.875", "5.5"),
            utils::resting("3.875", "5.5"),
            utils::planted("4.875", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // The walker keeps its quarter, 0.03125, and takes B's 0.09375, 0.125 in
    // all, leaning clockwise by a quarter of what it took, 0.0234375; B,
    // pressed dead-on, is nudged a quarter of its share aside, toward −y.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0.0234375"),
            utils::push("0", "-0.0234375"),
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A walker that takes over its partner's share still swerves on its own:
/// two walkers head-on, overlapping by 0.25, halve it and swerve; the one
/// flush against a wall makes none of its way, so the other takes its share
/// too and moves aside by a quarter of what it keeps plus a quarter of what
/// it takes.
#[test]
fn walker_taking_partner_share_keeps_its_swerve() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(6, 5), true);

    let pushes = utils::pushes_over(
        &grid,
        &[
            utils::walking("4.75", "5.5", "1", "0"),
            utils::walking("5.5", "5.5", "-1", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 0.125 each; the first keeps its 0.125 and takes the second's 0.125,
    // 0.25 back, and moves aside 0.125 / 4 + 0.125 / 4 = 0.0625, clockwise of
    // its way out (−x): toward +y.
    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0.0625"), FixedVec2::ZERO]
    );
}

/// The lean is a sine, never more than one, however short the heading and
/// the line it is read across: a walker a bit off its aim, its heading one
/// bit north, takes over the whole share of an idle body a bit west of it
/// flush against a wall, and leans by that share across the line.
#[test]
fn lean_holds_for_bit_long_heading_and_line() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(4, 5), true);
    let idle = utils::resting("5.5", "5.5");
    let walker = Body {
        location: Location::new(
            utils::point_bits(0x5_8000_0001, 0x5_8000_0000),
            CellSize::ONE,
            utils::MAP_SIZE,
        ),
        bearing: Bearing::Walking(Heading::Toward(FixedVec2::new(
            FixedI64::ZERO,
            FixedI64::from_bits(1),
        ))),
        ..utils::resting("5.5", "5.5")
    };

    let pushes = utils::pushes_over(&grid, &[walker, idle], |_, _| Meeting::Yielding);

    // The pair overlaps by all but a bit; the walker keeps its quarter and
    // takes the idle body's three quarters, and its heading, square to the
    // line between them, gives a sine of one: it leans by the whole three
    // quarters. Both parts held to half a cell per axis, the lean but for
    // its rounding.
    assert_eq!(
        pushes,
        vec![utils::push("0.5", "0.4999999998"), FixedVec2::ZERO]
    );
}

/// A body against the map's edge at an angle makes along the line exactly
/// what its way makes, and its walker takes exactly the rest: an idle body
/// flush against the west edge, pressed at 45° by a walker overlapping it
/// by 0.2, slides along the edge, and nothing of the overlap stays.
#[test]
fn body_sliding_on_edge_leaves_no_overlap() {
    let pushes = utils::pushes(
        &[
            utils::walking("1.0656854249", "4.9343145751", "-1", "1"),
            utils::resting("0.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 0.8 apart, 0.2 inside touching: the walker keeps 0.05 (three times as
    // stiff), the idle body's 0.15 makes half its way along the edge, so the
    // walker takes 0.075 more, 0.125 in all, leaning a quarter of what it
    // took, 0.01875: (0.125 − 0.01875) × √2 / 2 ≈ 0.0751 toward +x and
    // (0.125 + 0.01875) × √2 / 2 ≈ 0.1016 toward −y. The idle body, the
    // ground the tighter limit, is pushed its whole 0.15 and nudged a quarter
    // of it aside; the edge drops the x part, leaving ≈ (0.15 + 0.0375) × √2
    // / 2 ≈ 0.1326 toward +y, ≈ 0.094 along the line — the pair parts by ≈
    // 0.219, nothing of the 0.2 left.
    assert_eq!(
        pushes,
        vec![
            utils::push("0.0751300973", "-0.101646602"),
            utils::push("0", "0.1325825215")
        ]
    );
}

/// A body barely ahead caps nothing: an idle body pressed east by a walker
/// touches, all but square to the push, an idle body touching a planted one
/// dead-on behind it, so making none of its way; the cap it allows, a hair
/// below one, rounds up to the whole way.
#[test]
fn partner_barely_ahead_caps_nothing() {
    let pushes = utils::pushes(
        &[
            utils::walking("4.125", "5.5", "1", "0"),
            utils::resting("5", "5.5"),
            utils::resting("5.01", "6.4999500012"),
            utils::planted("5.02", "7.4999000024"),
        ],
        |_, _| Meeting::Yielding,
    );

    // The cap, 1 − 0.01 × (1 − 0) = 0.99, rounds up to the whole way: the
    // walker keeps only its own quarter of 0.125, 0.03125.
    assert_eq!(pushes[0], utils::push("-0.03125", "0"));
}

/// A share taken over is held to the taker's own way: idle A, B and C in a
/// row between planted P1 and P2, A overlapping P1 and B, C touching P2 —
/// B takes none of A's share east, its way that way making nothing, and the
/// overlap stays between A and B; A, squeezed, slides out aside.
#[test]
fn taken_share_is_held_to_taker_way() {
    let pushes = utils::pushes(
        &[
            utils::planted("2.375", "5.5"),
            utils::resting("3.25", "5.5"),
            utils::resting("4.125", "5.5"),
            utils::resting("5.125", "5.5"),
            utils::planted("6.125", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // A takes P1's whole 0.125 east, none of which it makes (B ahead makes
    // nothing), and its half of A–B, 0.0625, west, with B's half B cannot
    // make: 0.125 west into P1, which it already overlaps — met at once and
    // slid along P1's rim by a quarter, 0.03125, toward −y.
    assert_eq!(
        pushes,
        vec![
            FixedVec2::ZERO,
            utils::push("0", "-0.03125"),
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A body pressed into a wedge of two bodies that make none of their ways
/// is capped by each alone: idle B pressed east by a walker, two idle bodies
/// touching B at ±45° ahead, each touching a planted one dead-on behind it —
/// each allows 1 − cos 45° ≈ 0.29 of B's way.
#[test]
fn body_pressed_into_wedge_is_capped_by_each_partner() {
    let pushes = utils::pushes(
        &[
            utils::walking("4.625", "5.5", "1", "0"),
            utils::resting("5.5", "5.5"),
            utils::resting("6.2071067812", "4.7928932188"),
            utils::resting("6.2071067812", "6.2071067812"),
            utils::planted("6.9142135624", "4.0857864376"),
            utils::planted("6.9142135624", "6.9142135624"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Each allows 1 − 0.7071 ≈ 0.2929, rounded up to 19/64 = 0.296875; B
    // moves 0.09375 × 19/64 ≈ 0.0278 east, nudged a quarter of its share,
    // 0.0234375, toward −y; the walker keeps 0.03125 and takes 0.09375 ×
    // 45/64 ≈ 0.0659, ≈ 0.0972 in all, leaning a quarter of what it took.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.0971679688", "0.0164794922"),
            utils::push("0.0278320312", "-0.0234375"),
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A partner ahead at an angle caps a body's way by its cosine: idle B
/// pressed east by a walker touches, 60° off the push, an idle body touching
/// a planted one dead-on behind it, so making none of its way: B makes
/// 1 − cos 60° = 0.5 of its way, and the walker takes back the other half of
/// B's share.
#[test]
fn oblique_partner_caps_way_by_cosine() {
    let pushes = utils::pushes(
        &[
            utils::walking("4.125", "5.5", "1", "0"),
            utils::resting("5", "5.5"),
            utils::resting("5.5", "6.3660254037"),
            utils::planted("6", "7.2320508074"),
        ],
        |_, _| Meeting::Yielding,
    );

    // The walker keeps 0.03125 and takes back B's 0.09375 × 0.5 = 0.046875,
    // 0.078125 in all, leaning a quarter of what it took, 0.01171875.
    assert_eq!(pushes[0], utils::push("-0.078125", "0.01171875"));
}

/// A walker aiming nowhere leans past nothing: one with no aim yet takes
/// over the share of an idle body flush against a wall, and moves straight
/// back along the line.
#[test]
fn unaimed_taker_leans_nowhere() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(6, 5), true);
    let walker = Body {
        bearing: Bearing::Walking(Heading::Unaimed),
        ..utils::resting("4.625", "5.5")
    };

    let pushes = utils::pushes_over(&grid, &[walker, utils::resting("5.5", "5.5")], |_, _| {
        Meeting::Yielding
    });

    // Not pressing, it halves the 0.125 overlap with the idle body, 0.0625
    // each, and takes the idle body's half too — 0.125 straight back.
    assert_eq!(pushes[0], utils::push("-0.125", "0"));
}

/// A body is moved over the same ground its way is judged on: idle A
/// touches idle C, flush against a wall on its east; a walker pressing A
/// from the south-west pushes it north-east, and A slides along C's rim
/// rather than into it, C being shut toward A.
#[test]
fn body_never_moved_into_shut_neighbor() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(7, 5), true);
    let bodies = [
        utils::walking("4.88128", "6.11872", "1", "-1"),
        utils::resting("5.5", "5.5"),
        utils::resting("6.5", "5.5"),
    ];

    let pushes = utils::pushes_over(&grid, &bodies, |_, _| Meeting::Yielding);

    // A's sliver north-east makes half its way along the line; the ground is
    // the tighter limit, so A is pushed its whole 0.09375 north-east, nudged
    // a quarter of that aside — pressed dead-on — and meets C at once,
    // keeping only its northward part, ≈ (0.09375 + 0.0234375) × √2 / 2 ≈
    // 0.0829, never east into it. The walker takes back 0.09375 × 0.5 =
    // 0.046875 with its own 0.03125, 0.078125 to the south-west, and leans
    // clockwise by a quarter of what it took, ≈ 0.0117: (0.078125 −
    // 0.01171875) × √2 / 2 ≈ 0.0470 and (0.078125 + 0.01171875) × √2 / 2 ≈
    // 0.0635.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.0469554774", "0.063527999"),
            utils::push("0", "-0.0828626074"),
            FixedVec2::ZERO
        ]
    );
}

/// A body's way keeps its ground's and its chain's parts apart: inert B,
/// pressed north-east by a walker, has inert D east of it, sliding along a
/// planted body, and inert C north of it, in a gap one cell wide under inert
/// E, flush against a wall. E stands, so C, wedged, stands too; B's ground
/// part is its slide along C's rim, and D caps its chain part to about the
/// same. B makes along the line exactly what its walker leaves it, nothing
/// of the overlap staying.
#[test]
fn way_keeps_its_parts_when_partner_turns_shut() {
    let mut grid = utils::open_grid();
    for (x, y) in [(5, 6), (7, 6), (6, 8)] {
        grid.set_occupied(utils::GROUND, CellPos::new(x, y), true);
    }
    let bodies = [
        utils::walking("5.8812815665", "4.8812815665", "1", "0"),
        utils::inert("6.5", "5.5"),
        utils::inert("6.5", "6.5"),
        utils::inert("6.5", "7.5"),
        utils::inert("7.5", "5.5"),
        utils::planted("8.3660254038", "6"),
    ];

    let pushes = utils::pushes_over(&grid, &bodies, |_, _| Meeting::Yielding);

    // 0.875 apart, 0.125 inside touching; the walker, pressing, keeps 0.03125
    // and B's share is 0.09375. D slides a quarter of its way east, so B's
    // chain allows 1 − cos 45° × 0.75 ≈ 0.4697, rounded up to 31/64; sliding
    // along C its ground allows ≈ 0.5. B makes 31/64 of its share along the
    // line, 0.09375 × 31/64 ≈ 0.0454: pushed (31/64) / 0.5 = 31/32 of its
    // share, ≈ 0.0908 north-east, C's rim dropping its north part, ≈ 0.0642
    // along x. The walker takes back the rest, 0.09375 × 33/64 ≈ 0.0483, with
    // its own, ≈ 0.0796 to the south-west, leaning past B on what it took,
    // 45° off its heading: ≈ 0.0483 × sin 45° ≈ 0.0342 — (−(0.0796 −
    // 0.0342), −(0.0796 + 0.0342)) × √2 / 2 ≈ (−0.0321, −0.0804).
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.032108596", "-0.0804484403"),
            utils::push("0.0642196606", "0"),
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A way reads a partner that turns wall as a wall: walker W presses inert
/// B, which touches inert C 60° off its way; C touches inert D, flush on a
/// wall east, and the row above is walled. D stands, so C stands against it,
/// and B's way, judged against those final walls, slides along C's rim.
#[test]
fn way_reads_partner_turned_wall() {
    let pushes = utils::pushes_over(&walled_pocket(), &pocket_bodies(), |_, _| Meeting::Yielding);

    // W, pressing, keeps 0.03125 of the 0.125 overlap; B's share is 0.09375.
    // D and C stand, so B slides along C's rim, 60° off its line, making
    // sin² 60° = 3/4 of its share along it. W takes back the other quarter,
    // 0.0234375, with its own, 0.0546875 back along its line and leaning past
    // B: ≈ (−0.0521, −0.0176). B, pushed its whole share and nudged aside a
    // quarter of it, slides along C's rim to ≈ (0.0663, −0.0208).
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.05210998", "-0.0175955896"),
            utils::push("0.066271236", "-0.0208413396"),
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A cap one link removed reads the final walls: inert A, pressed by walker
/// W, presses dead-on into B of the pocket; C stands as a wall, B slides
/// along its rim, and A's cap, read from B's way, follows though A's own
/// walls hold no wall.
#[test]
fn cap_one_link_removed_reads_final_walls() {
    let mut bodies = pocket_bodies().to_vec();
    bodies[0] = utils::walking(
        "4.3697457836",
        "4.1388820868",
        "0.9761355821",
        "0.2171619796",
    );
    bodies.push(utils::inert("5.2238644179", "4.328898819"));

    let pushes = utils::pushes_over(&walled_pocket(), &bodies, |_, _| Meeting::Yielding);

    // A, 0.125 inside touching W, has the 0.09375 share; B dead ahead caps
    // it at what B makes, 1 − 1 × (1 − made), with C a wall: B's rim
    // slide makes a hair over 3/4, rounded up to 49/64. A makes 0.09375 ×
    // 49/64 ≈ 0.0718 along the line, nudged aside a quarter of its share,
    // 0.0234375: ≈ (0.0752, −0.0073). W takes back 0.09375 × 15/64 ≈ 0.022
    // with its own 0.03125, ≈ 0.0532 back along its line: ≈ (−0.0531,
    // −0.0062). Nothing touching B overlaps it, so B stays.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.0531454345", "-0.0061958646"),
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            FixedVec2::ZERO,
            utils::push("0.075154153", "-0.0072908679")
        ]
    );
}

/// A shut partner is a wall only within a sliver of touching: idle B, 0.2
/// from idle P flush against a wall, is pushed east by a walker overlapping it
/// deeply; P, shut toward B, is too far to be a wall, so B's push carries it
/// into P as it would were B walking.
#[test]
fn shut_partner_beyond_sliver_is_no_wall() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(7, 5), true);
    let pushed = |b: Body| {
        utils::pushes_over(
            &grid,
            &[
                utils::walking("4.9", "5.5", "1", "0"),
                b,
                utils::resting("6.5", "5.5"),
            ],
            |_, _| Meeting::Yielding,
        )
    };

    // The walker, pressing, keeps a quarter of the 0.6 overlap, 0.15; B is
    // pushed 0.45 east, nudged a quarter of that, 0.1125 (but for the last
    // bits), aside, and ends 0.25 past P's rim along x rather than stopping
    // on it.
    assert_eq!(
        pushed(utils::resting("5.3", "5.5")),
        vec![
            utils::push("-0.15", "0"),
            utils::push("0.45", "-0.1124999998"),
            FixedVec2::ZERO
        ]
    );
    // Walking, B is nudged nowhere and pushed the same 0.45 into P.
    assert_eq!(
        pushed(utils::walking("5.3", "5.5", "1", "0")),
        vec![
            utils::push("-0.15", "0"),
            utils::push("0.45", "0"),
            FixedVec2::ZERO
        ]
    );
}

//
// ─── Drift and bounds ─────────────────────────────────────────────────────────
//

/// A drift toward a shut side is dropped, and the rest is taken: idle B,
/// flush against a wall on its east, touches idle neighbors on its west and
/// north; the west one's drift would carry B into the wall, so B takes only
/// the north one's, away from it.
#[test]
fn drift_toward_shut_side_is_dropped() {
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(7, 5), true);

    let pushes = utils::pushes_over(
        &grid,
        &[
            utils::resting("6.5", "5.5"),
            utils::resting("5.5", "5.5"),
            utils::resting("6.5", "4.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Touching, 0.25 short of the spacing: one pass's drift, 0.125 each.
    assert_eq!(
        pushes,
        vec![
            utils::push("0", "0.125"),
            utils::push("-0.125", "0"),
            utils::push("0", "-0.125")
        ]
    );
}

/// A body drifted from both sides is squeezed though one side is shut: in a
/// corridor one cell high, idle B between idle A and idle C, C flush against
/// a wall east; B cannot drift east from A, but A still presses it from the
/// west, so B takes no drift from C either, and only A drifts away.
#[test]
fn body_drifted_from_both_sides_is_squeezed_though_one_is_shut() {
    let mut grid = utils::corridor_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(8, 5), true);

    let pushes = utils::pushes_over(
        &grid,
        &[
            utils::resting("5.5", "5.5"),
            utils::resting("6.5", "5.5"),
            utils::resting("7.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Touching, 0.25 short of the spacing: 0.125 each a pass. C is shut
    // against the wall, so B is shut east; B, drifted from the west by A and
    // from the east by C, 180° apart, is squeezed; A drifts its 0.125 west.
    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), FixedVec2::ZERO, FixedVec2::ZERO]
    );
}

/// A drift is held to its way as a share is: idle X, touching idle Y on its
/// west, drifts east toward inert Z, which makes none of its way, pressed
/// dead-on against a planted body — so X drifts only what Z lets it, not into
/// Z.
#[test]
fn drift_is_held_to_its_way() {
    let pushes = utils::pushes(
        &[
            utils::resting("5.5", "5.5"),
            utils::resting("4.5", "5.5"),
            utils::inert("6.5", "5.5"),
            utils::planted("7.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // X and Y touch, 0.25 short of the spacing: 0.125 each. Z, pressed
    // dead-on, makes none of its way, so X's chain allows 1 − 1 × (1 − 0) = 0
    // of its own: X is moved by none of its drift; Y drifts its whole 0.125
    // west.
    assert_eq!(
        pushes,
        vec![
            FixedVec2::ZERO,
            utils::push("-0.125", "0"),
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// A drift is held to its way at any spacing: idle X, 0.1 from idle Y on its
/// west, drifts east toward inert Z, which makes none of its way, pressed
/// dead-on against a planted body — so X drifts none of it, as it would
/// touching Y.
#[test]
fn drift_is_held_to_its_way_at_any_spacing() {
    let pushes = utils::pushes(
        &[
            utils::resting("5.5", "5.5"),
            utils::resting("4.4", "5.5"),
            utils::inert("6.5", "5.5"),
            utils::planted("7.5", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 0.1 apart, 0.25 − 0.1 = 0.15 short of the spacing: 0.075 each. Z makes
    // none of its way, so X's chain allows none of its own: X stays; Y
    // drifts its whole 0.075 west.
    assert_eq!(
        pushes,
        vec![
            FixedVec2::ZERO,
            utils::push("-0.075", "0"),
            FixedVec2::ZERO,
            FixedVec2::ZERO
        ]
    );
}

/// Two drifts 108° apart, as in a ring of five, do not squeeze a body: it
/// drifts outward between them.
#[test]
fn drifts_108_degrees_apart_are_taken() {
    let pushes = utils::pushes(
        &[
            utils::resting("10", "10"),
            utils::resting("9.4115", "9.1905"),
            utils::resting("9.4115", "10.8095"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Each neighbor 1.0008 away, 0.2492 short of the spacing, 0.1246 each;
    // the two at ±54° from the x axis sum to 2 × 0.1246 × cos 54° ≈ 0.1465
    // along it, their cross parts canceling.
    assert_eq!(pushes[0], utils::push("0.1465289495", "0.0000000002"));
}

/// Two drifts more than 120° apart squeeze a body: neighbors 130° apart
/// around it leave it where it stands while they drift away.
#[test]
fn drifts_130_degrees_apart_squeeze_body() {
    let pushes = utils::pushes(
        &[
            utils::resting("10", "10"),
            utils::resting("9.577", "9.0927"),
            utils::resting("9.577", "10.9073"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Each neighbor at (−0.423, ∓0.9073), √(0.423² + 0.9073²) ≈ 1.0011 away,
    // 0.2489 short of the spacing, ±65° from −x — 130° apart: the middle body
    // takes no drift, and each neighbor drifts its own half, ≈ 0.1245, away
    // from it (the two differ in the last bits of the rounding).
    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], utils::push("-0.0525949057", "-0.1128117207"));
    assert_eq!(pushes[2], utils::push("-0.0525949057", "0.1128117205"));
}

/// A weightless neighbor takes the whole drift and never squeezes the body it
/// drifts from: three idle bodies 1.125 apart, the left weightless — it
/// drifts 0.125 away, and the middle and right halve their 0.125 shortfall.
#[test]
fn weightless_neighbor_takes_whole_drift() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.875", "5.5"), "0"),
            utils::resting("4", "5.5"),
            utils::resting("5.125", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Shortfall 1 + 0.25 − 1.125 = 0.125 on each side.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0"),
            utils::push("-0.0625", "0"),
            utils::push("0.0625", "0")
        ]
    );
}

/// Bodies on disjoint layers never meet, overlap as they may.
#[test]
fn bodies_on_disjoint_layers_never_meet() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::flying("5.25", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
}

/// Two bodies set on one point part along x, the lower index west, each
/// push held to half a cell: their whole-cell overlap, half each.
#[test]
fn coincident_bodies_part_along_x_by_index() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::resting("5", "5.5")],
        |_, _| Meeting::Yielding,
    );

    // Overlap 1, half each, held at the 0.5 cap.
    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0"), utils::push("0.5", "0")]
    );
}

/// Two bodies a bit apart on each axis part along their line by no more than
/// their shares: the line's unit is a unit, however short the offset.
#[test]
fn bodies_bit_apart_part_by_their_shares() {
    // 5 and 5.5 plus 2⁻³² each.
    let pushes = utils::pushes(
        &[
            utils::resting("5", "5.5"),
            utils::resting(
                "5.00000000023283064365386962890625",
                "5.50000000023283064365386962890625",
            ),
        ],
        |_, _| Meeting::Yielding,
    );

    // Overlap 1 less a speck, half each, along 45°: 0.5 × √2 / 2 ≈ 0.35355
    // on each axis, inside the 0.5 cap — not the cap's 0.5 on each, which a
    // unit floored from a length of one bit would give.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.3535533901", "-0.3535533901"),
            utils::push("0.3535533901", "0.3535533901")
        ]
    );
}

//
// ─── Any size ─────────────────────────────────────────────────────────────────
//

/// The rules hold at any radius: two idle radius-0.05 specks 0.0375 apart,
/// meeting unyielding, halve their 0.1 − 0.0375 = 0.0625 overlap, 0.03125
/// each.
#[test]
fn specks_halve_overlap() {
    let speck = |x: &str| Body {
        radius: utils::cells("0.05"),
        ..utils::resting(x, "5.5")
    };
    let pushes = utils::pushes(&[speck("5"), speck("5.0375")], |_, _| Meeting::Unyielding);

    assert_eq!(
        pushes,
        vec![utils::push("-0.03125", "0"), utils::push("0.03125", "0")]
    );
}

/// A radius-4 body and a radius-0.5 one 4 apart overlap by 4.5 − 4 = 0.5 and
/// halve it, being of one weight, 0.25 each.
#[test]
fn giant_and_small_body_halve_overlap() {
    let pushes = utils::pushes(
        &[utils::wide("20", "20", 8), utils::resting("24", "20")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

/// A giant does not give way to a walker it outranks either: a radius-4
/// planted body leaves a small walker 0.5 into it the whole overlap.
#[test]
fn small_walker_takes_whole_overlap_from_planted_giant() {
    let giant = Body {
        bearing: Bearing::Planted,
        ..utils::wide("20", "20", 8)
    };
    let pushes = utils::pushes(&[giant, utils::walking("16", "20", "1", "0")], |_, _| {
        Meeting::Yielding
    });

    // 20 − 16 = 4 apart, 0.5 inside the touching 4 + 0.5 = 4.5.
    assert_eq!(pushes, vec![FixedVec2::ZERO, utils::push("-0.5", "0")]);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// An open grid with row 6 walled from column 0 to 12 and the cell (8, 5):
/// a pocket under a wall, closed on its east.
fn walled_pocket() -> NavGrid {
    let mut grid = utils::open_grid();
    for x in 0..13 {
        grid.set_occupied(utils::GROUND, CellPos::new(x, 6), true);
    }
    grid.set_occupied(utils::GROUND, CellPos::new(8, 5), true);
    grid
}

/// Walker W pressing inert B, 0.125 inside touching; inert C touching B 60°
/// off W's line and touching inert D, flush on the pocket's east wall.
fn pocket_bodies() -> [Body; 4] {
    [
        utils::walking(
            "5.3458813657",
            "4.3560440665",
            "0.9761355821",
            "0.2171619796",
        ),
        utils::inert("6.2", "4.546060798583"),
        utils::inert("6.5", "5.5"),
        utils::inert("7.5", "5.5"),
    ]
}

/// How a pair meets when only the body at index `unyielding` meets its
/// partners unyielding: so with it, yielding otherwise.
fn unyielding_with(unyielding: usize, second: usize) -> Meeting {
    if second == unyielding {
        Meeting::Unyielding
    } else {
        Meeting::Yielding
    }
}
