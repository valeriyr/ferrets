//! The pairwise contact rules: who swerves, who parts radially, who drifts
//! to the idle spacing, and how hard anyone may be shoved.

mod utils;

use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};
use ferrets_physics::{
    body::{Bearing, Body},
    contact::{self, Meeting},
};

//
// ─── Swerve ───────────────────────────────────────────────────────────────────
//

/// Head-on walkers press into each other mutually — the lattice's common
/// case. Each swerves to its own side on top of the radial part:
///
///   A →   ← B      pushes: A ↙  B ↗
#[test]
fn head_on_pair_swerves_apart() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::walking("3", "5.5", "-1", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Overlap one half → a quarter cell radially each, and a quarter of that
    // sideways, 0.25 / 4 = 0.0625, each to its own side.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.25", "0.0625"),
            utils::push("0.25", "-0.0625")
        ]
    );
}

/// A walker pressing into an idle body dead-on nudges it aside: three
/// quarters of the overlap along the line, and a quarter of that clockwise of
/// its way out; the walker keeps its line.
#[test]
fn walker_pressing_idle_body_nudges_it_aside() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::resting("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.375", "-0.09375")],
        "an idle body yields three quarters of the separation to the walker \
         pressing it, 0.375, and steps a quarter of that aside, 0.09375"
    );
}

/// An inert body is nudged aside as an idle one is: pressed dead-on, it
/// takes three quarters of the overlap along the line and a quarter of that
/// clockwise of its way out.
#[test]
fn walker_pressing_inert_body_nudges_it_aside() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::inert("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 1 − 0.5 = 0.5 overlap; the walker, pressing (three times as stiff),
    // keeps 0.5 / 4 = 0.125, the inert body 0.375, nudged 0.375 / 4 =
    // 0.09375 toward −y.
    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.375", "-0.09375")]
    );
}

/// The nudge tapers off the walker's heading: a walker heading 20° off an
/// idle body nudges it by a quarter of its share times one less the sine
/// over a half, toward the side it lies off the heading; one heading 45° off
/// nudges it not at all; one heading away nudges nothing.
#[test]
fn nudge_tapers_off_walker_heading() {
    let nudge = |toward_x: &str, toward_y: &str| {
        utils::pushes(
            &[
                utils::walking("2.5", "5.5", toward_x, toward_y),
                utils::resting("3", "5.5"),
            ],
            |_, _| Meeting::Yielding,
        )[1]
    };

    // 20° off: sin 20° ≈ 0.342, a quarter of 0.375 times 1 − 0.342 / 0.5 ≈
    // 0.0296, toward −y, the side the body lies off a heading toward +y.
    assert_eq!(
        nudge("1", "0.3639702343"),
        utils::push("0.375", "-0.029621223")
    );
    // 45° off: past 30°, no nudge. Heading away: not pressing, the pair halves
    // the 0.5 overlap, and no nudge.
    assert_eq!(nudge("1", "1"), utils::push("0.375", "0"));
    assert_eq!(nudge("-1", "0"), utils::push("0.25", "0"));
}

/// Coincident bodies — opposing walks stepping onto the same point in one
/// tick — get no direction from the press test, but any walker in the stack
/// must still swerve out or the pair re-converges forever.
#[test]
fn coincident_walkers_separate() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::walking("2.5", "5.5", "-1", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    // A full-body overlap parts along the x axis by pair order, swerving.
    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0.125"), utils::push("0.5", "-0.125")]
    );
}

/// Walkers converging on one point have nothing to pass, so they part
/// straight: each heads for a point between them, short of the other body.
#[test]
fn converging_walkers_part_without_swerve() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "0.25", "0"),
            utils::walking("3", "5.5", "-0.25", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    // 0.5 inside the touching 1; 0.25 each, straight.
    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

//
// ─── Radial contacts ──────────────────────────────────────────────────────────
//

/// Companions marching the same way press one-sidedly at most: the follower
/// toward the leader, the leader away — so the leader takes the whole
/// separation, bumped along its own way. A sideways share under their
/// sustained contact would roll them around each other, so no swerve.
#[test]
fn same_way_pair_parts_radially() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::walking("3", "5.5", "1", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.375", "0")],
        "same-way companions must not swerve around each other"
    );
}

/// A settled pile presses not at all: overlapping idle bodies part along the
/// straight line between them, so the pile relaxes instead of milling.
#[test]
fn resting_pair_parts_radially() {
    let pushes = utils::pushes(
        &[utils::resting("2.5", "5.5"), utils::resting("3", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

//
// ─── Weight ───────────────────────────────────────────────────────────────────
//

/// Bodies in the same state part in proportion to what they weigh: the
/// heavier one yields less than the lighter it meets.
#[test]
fn heavier_resting_body_yields_less_than_lighter() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.5", "5.5"), "1"),
            utils::weighing(utils::resting("3", "5.5"), "3"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Three quarters of the half-cell overlap to the light body, a quarter to
    // the body weighing three times as much.
    assert_eq!(
        pushes,
        vec![utils::push("-0.375", "0"), utils::push("0.125", "0")]
    );
}

/// Pressing into a contact resists as three times the same body at rest, so a
/// walker keeps its line until what it meets outweighs it by more than that.
/// Far past it, the roles invert: the idle body stands and the walker is the
/// one shoved aside.
#[test]
fn walker_pressing_far_heavier_idle_body_takes_larger_share() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::walking("2.5", "5.5", "1", "0"), "1"),
            utils::weighing(utils::resting("3", "5.5"), "9"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.375", "0"), utils::push("0.125", "-0.03125")],
        "nine times the walker's weight inverts the shares an equal pair takes; \
         the idle body still steps a quarter of its 0.125 aside"
    );
}

/// The other end: a walker heavy enough plows a light idle body aside — its
/// 15/16 of the half-cell overlap, and a quarter of that sideways — and
/// barely gives ground itself.
#[test]
fn heavy_walker_plows_light_idle_body_aside() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::walking("2.5", "5.5", "1", "0"), "5"),
            utils::resting("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.03125", "0"),
            utils::push("0.46875", "-0.1171875")
        ]
    );
}

/// The press stiffening cancels wherever both bodies are in the same state, so
/// a head-on meeting is a pure weight contest — the shares two resting bodies
/// of these weights take, with the head-on swerve on top.
#[test]
fn mutual_press_splits_by_weight_alone() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::walking("2.5", "5.5", "1", "0"), "1"),
            utils::weighing(utils::walking("3", "5.5", "-1", "0"), "3"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.375", "0.09375"),
            utils::push("0.125", "-0.03125")
        ],
        "the radial shares must match the 0.375/0.125 two resting bodies \
         weighing 1 and 3 take"
    );
}

/// A weight split fixed point cannot hold exactly still parts the pair by the
/// whole overlap: the yielding body takes whatever the other's share leaves.
#[test]
fn truncated_weight_split_parts_by_full_separation() {
    // Weights 1 and 2 put the shares at two thirds and one third of the
    // overlap, neither of which fixed point holds exactly.
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.5", "5.5"), "1"),
            utils::weighing(utils::resting("3", "5.5"), "2"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes[1].x - pushes[0].x,
        utils::signed_cells("0.5"),
        "the pair must part by the whole overlap however the split truncates"
    );
}

/// Equals part by mirror-image shoves, so which body a pass happens to walk
/// first cannot decide who gives ground.
#[test]
fn equal_weights_part_symmetrically_whichever_order() {
    let pushes = utils::pushes(
        &[utils::resting("2.5", "5.5"), utils::resting("3", "5.5")],
        |_, _| Meeting::Yielding,
    );
    let reversed = utils::pushes(
        &[utils::resting("3", "5.5"), utils::resting("2.5", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes[0], -pushes[1]);
    assert_eq!(pushes[0], reversed[1]);
    assert_eq!(pushes[1], reversed[0]);
}

/// A body debuffed to weightlessness has nothing to weigh against what it
/// meets, so it yields the whole separation and the other stands.
#[test]
fn weightless_body_yields_whole_separation() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.5", "5.5"), "0"),
            utils::weighing(utils::resting("3", "5.5"), "1"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0"), utils::push("0", "0")]
    );
}

/// Bodies debuffed to weightlessness have nothing left to weigh against each
/// other, so they fall back on parting evenly.
#[test]
fn weightless_bodies_part_evenly() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.5", "5.5"), "0"),
            utils::weighing(utils::resting("3", "5.5"), "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

/// Pressing triples what a body weighs, so a third of the fixed-point range is
/// the most the stiffening carries before it saturates. Either side of that
/// bound resolves the same way, and resolves at all: nothing bounds an authored
/// weight, and wrapping one would turn resistance negative — inverting the
/// contest, and doing it only in the builds that do not panic first.
#[test]
fn weight_past_stiffening_range_saturates_rather_than_wrapping() {
    let against_idle = |weight: &str| {
        utils::pushes(
            &[
                utils::weighing(utils::walking("2.5", "5.5", "1", "0"), weight),
                utils::resting("3", "5.5"),
            ],
            |_, _| Meeting::Yielding,
        )
    };

    // One weight past the third saturates the stiffening; the heaviest a weight
    // can even be authored saturates the conversion into it as well. Both stand
    // at the same ceiling, so the shares stop moving instead of wrapping.
    let saturated = against_idle("715827883");
    assert_eq!(saturated, against_idle("4000000000"));

    // Outweighed by the whole range, the walker gives up a single bit of the
    // half-cell overlap and the idle body takes everything left, stepping a
    // quarter of it aside.
    let bit = FixedI64::from_bits(1);
    let idle = utils::signed_cells("0.5") - bit;
    assert_eq!(
        saturated,
        vec![
            FixedVec2 {
                x: -bit,
                y: FixedI64::ZERO,
            },
            FixedVec2 {
                x: idle,
                y: -(idle / FixedI64::from_num(4)),
            },
        ]
    );
}

/// Pressing stiffens a body by what it weighs, so a weightless one presses
/// with nothing: it yields the whole separation walking into an idle body,
/// exactly as it would meeting one at rest.
#[test]
fn weightless_walker_yields_whole_separation() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::walking("2.5", "5.5", "1", "0"), "0"),
            utils::resting("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0"), utils::push("0", "0")],
        "the walker takes the whole separation, straight back"
    );
}

//
// ─── Idle spacing ─────────────────────────────────────────────────────────────
//

/// Two idle bodies touching drift apart toward the 0.25 idle spacing: the
/// whole shortfall, 0.25, is one pass's drift, an eighth of a cell each.
#[test]
fn idle_pair_drifts_toward_its_spacing() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::resting("6", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.125", "0")]
    );
}

/// The drift closes the shortfall and no more: 0.125 apart, 0.125 short of
/// the spacing, a sixteenth of a cell each.
#[test]
fn idle_drift_stops_at_its_spacing() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::resting("6.125", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.0625", "0"), utils::push("0.0625", "0")]
    );
}

/// Idle bodies the spacing apart, or short of it by no more than the slack
/// of 1/64, rest: 1.25 and 1.25 − 1/128 apart.
#[test]
fn idle_pair_within_its_slack_rests() {
    for far in ["6.25", "6.2421875"] {
        let pushes = utils::pushes(
            &[utils::resting("5", "5.5"), utils::resting(far, "5.5")],
            |_, _| Meeting::Yielding,
        );

        assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO], "{far}");
    }
}

/// The slack's boundary: idle bodies exactly 1/64 short of the spacing,
/// 1.25 − 1/64 = 1.234375 apart, rest; 1/1024 closer they drift the whole
/// shortfall, 1/64 + 1/1024 = 0.0166015625, half of it each.
#[test]
fn idle_pair_rests_at_its_slack_and_drifts_past_it() {
    let at_slack = utils::pushes(
        &[
            utils::resting("5", "5.5"),
            utils::resting("6.234375", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );
    let past_slack = utils::pushes(
        &[
            utils::resting("5", "5.5"),
            utils::resting("6.2333984375", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(at_slack, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
    assert_eq!(
        past_slack,
        vec![
            utils::push("-0.00830078125", "0"),
            utils::push("0.00830078125", "0")
        ]
    );
}

/// A sliver of overlap between idle bodies drifts them as far as touching
/// ones, never by the sliver alone: 1/1024 of overlap parts by the 0.25
/// drift, an eighth of a cell each.
#[test]
fn idle_sliver_of_overlap_drifts_as_touching_does() {
    let pushes = utils::pushes(
        &[
            utils::resting("5", "5.5"),
            utils::resting("5.9990234375", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.125", "0")]
    );
}

/// A body drifted from opposing sides takes no drift: between idle bodies
/// 1 and 1.1 away, it would drift 0.125 − 0.075 = 0.05 one way; squeezed, it
/// rests, while the two outer bodies drift out, 0.125 and 0.075.
#[test]
fn squeezed_idle_body_takes_no_drift() {
    let pushes = utils::pushes(
        &[
            utils::resting("4", "5.5"),
            utils::resting("5", "5.5"),
            utils::resting("6.1", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0"),
            FixedVec2::ZERO,
            utils::push("0.075", "0")
        ]
    );
}

/// A squeezed body still parts what it overlaps: 0.1 into its left
/// neighbor, it takes half the overlap, 0.05, and none of the drift.
#[test]
fn squeezed_body_still_parts_its_overlap() {
    let pushes = utils::pushes(
        &[
            utils::resting("4", "5.5"),
            utils::resting("4.9", "5.5"),
            utils::resting("5.9", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Left pair, 0.9 apart: overlap 0.1 parted half each, 0.05, and drift
    // to 0.25 past it, 0.15, half each, 0.075. Right pair, touching: drift
    // 0.25, half each, 0.125. The left body takes −0.05 − 0.075 = −0.125;
    // the middle, pushed from both sides, keeps only its overlap's +0.05; the
    // right takes its drift, +0.125. The outer pair, 1.9 apart, is too far
    // to meet.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0"),
            utils::push("0.05", "0"),
            utils::push("0.125", "0")
        ]
    );
}

/// A drift never drives a body into an overlap: overlapped 0.05 by an inert
/// body on one side and drifted toward it from the other, the middle
/// body takes half the overlap, +0.025, and none of the −0.075 drift, which
/// would carry it 0.05 deeper.
#[test]
fn drift_never_drives_body_into_overlap() {
    let pushes = utils::pushes(
        &[
            utils::inert("4", "5.5"),
            utils::resting("4.95", "5.5"),
            utils::resting("6.05", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Left pair: overlap 0.05, half each, 0.025, no spacing beside an inert
    // body. Right pair, 1.1 apart: drift 0.15, half each, 0.075.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.025", "0"),
            utils::push("0.025", "0"),
            utils::push("0.075", "0")
        ]
    );
}

/// A pair with a working body in it parts only on overlap: two touching
/// working bodies, a working body beside an idle one, and a walker walking
/// away from an idle one all rest.
#[test]
fn working_pair_parts_only_on_overlap() {
    for pair in [
        [utils::working("5", "5.5"), utils::working("6", "5.5")],
        [utils::working("5", "5.5"), utils::resting("6", "5.5")],
        [
            utils::walking("5", "5.5", "-1", "0"),
            utils::resting("6", "5.5"),
        ],
    ] {
        let pushes = utils::pushes(&pair, |_, _| Meeting::Yielding);

        assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
    }
}

//
// ─── Planted bodies ───────────────────────────────────────────────────────────
//

/// A walker pressing into a planted body takes the whole separation,
/// straight back; the planted body stays where it is.
#[test]
fn walker_pressing_planted_body_takes_whole_separation() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::planted("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Overlap one half, all of it the walker's.
    assert_eq!(pushes, vec![utils::push("-0.5", "0"), FixedVec2::ZERO]);
}

/// Weight earns nothing against a planted body: a resting body nine times
/// its weight takes the whole half-cell overlap.
#[test]
fn heavier_body_yields_whole_separation_to_planted_one() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::resting("2.5", "5.5"), "9"),
            utils::planted("3", "5.5"),
        ],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes, vec![utils::push("-0.5", "0"), FixedVec2::ZERO]);
}

/// A planted body never moves for a working one: planted and working
/// bodies overlapping by 0.5 meeting yielding, the working body takes it all.
#[test]
fn planted_body_never_moves_for_working_one() {
    let pushes = utils::pushes(
        &[utils::planted("2.5", "5.5"), utils::working("3", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes, vec![FixedVec2::ZERO, utils::push("0.5", "0")]);
}

/// Two planted bodies overlapping part as two working bodies do, half the
/// overlap each: 0.5 / 2 = 0.25.
#[test]
fn planted_pair_parts_as_working_pair_does() {
    let pushes = utils::pushes(
        &[utils::planted("2.5", "5.5"), utils::planted("3", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

/// A planted body takes no idle spacing: touching an idle body, or a planted
/// one, neither drifts.
#[test]
fn planted_body_never_drifts() {
    for pair in [
        [utils::planted("5", "5.5"), utils::resting("6", "5.5")],
        [utils::planted("5", "5.5"), utils::planted("6", "5.5")],
    ] {
        let pushes = utils::pushes(&pair, |_, _| Meeting::Yielding);

        assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
    }
}

/// A planted body outranks the meeting: an idle body meeting it unyielding
/// and overlapping it by half a cell takes the whole of it, where two idle
/// bodies meeting unyielding halve.
#[test]
fn planted_body_outranks_unyielding_meeting() {
    let pushes = utils::pushes(
        &[utils::resting("2.5", "5.5"), utils::planted("3", "5.5")],
        |_, _| Meeting::Unyielding,
    );

    assert_eq!(pushes, vec![utils::push("-0.5", "0"), FixedVec2::ZERO]);
}

/// A fixed body outranks a planted one: the planted body takes the whole
/// half-cell overlap.
#[test]
fn planted_body_takes_whole_separation_from_fixed_body() {
    let pushes = utils::pushes(
        &[utils::planted("2.5", "5.5"), utils::fixed("3", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes, vec![utils::push("-0.5", "0"), FixedVec2::ZERO]);
}

/// Two fixed bodies take nothing from each other, whatever they overlap.
#[test]
fn fixed_pair_takes_nothing() {
    let pushes = utils::pushes(
        &[utils::fixed("2.5", "5.5"), utils::fixed("3", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
}

/// Weights past anything authored still split without overflow: two
/// radius-2 bodies of weights 1.2 and 0.9 billion overlapping by 3 share it
/// 0.9 / 2.1 and 1.2 / 2.1, where measuring the product first would carry
/// 3 × 0.9 billion past the type. Only the absence of an overflow is seen:
/// both shares, 9/7 and 12/7, are held to the half-cell cap.
#[test]
fn absurd_weights_part_without_overflow() {
    let heavy = Body {
        radius: utils::cells("2"),
        ..utils::weighing(utils::resting("4", "5.5"), "1200000000")
    };
    let light = Body {
        radius: utils::cells("2"),
        ..utils::weighing(utils::resting("5", "5.5"), "900000000")
    };

    let pushes = utils::pushes(&[heavy, light], |_, _| Meeting::Yielding);

    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0"), utils::push("0.5", "0")]
    );
}

//
// ─── Unyielding meetings ──────────────────────────────────────────────────────
//

/// A walker meeting a body that will not yield to it takes the whole
/// separation, straight back, as against a planted body.
#[test]
fn walker_meeting_unyielding_body_takes_whole_separation() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::resting("3", "5.5"),
        ],
        |_, _| Meeting::Unyielding,
    );

    // Overlap one half, all of it the walker's.
    assert_eq!(pushes, vec![utils::push("-0.5", "0"), FixedVec2::ZERO]);
}

/// Unyielding walkers halve the separation whatever they weigh: a walker
/// nine times its rival's weight parts as head-on equals do.
#[test]
fn unyielding_walkers_halve_separation_whatever_weights() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::walking("2.5", "5.5", "1", "0"), "9"),
            utils::walking("3", "5.5", "-1", "0"),
        ],
        |_, _| Meeting::Unyielding,
    );

    // Overlap one half → a quarter each, a quarter of that sideways:
    // 0.25 × 1/4 = 0.0625.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.25", "0.0625"),
            utils::push("0.25", "-0.0625")
        ]
    );
}

/// Two working bodies meeting unyielding halve an overlap whatever they
/// weigh.
#[test]
fn unyielding_working_pair_halves_separation_whatever_weights() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::working("2.5", "5.5"), "9"),
            utils::working("3", "5.5"),
        ],
        |_, _| Meeting::Unyielding,
    );

    // 0.5 / 2 = 0.25 each.
    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

/// Two planted bodies meeting unyielding halve an overlap whatever they
/// weigh.
#[test]
fn unyielding_planted_pair_halves_separation_whatever_weights() {
    let pushes = utils::pushes(
        &[
            utils::weighing(utils::planted("2.5", "5.5"), "9"),
            utils::planted("3", "5.5"),
        ],
        |_, _| Meeting::Unyielding,
    );

    // 0.5 / 2 = 0.25 each.
    assert_eq!(
        pushes,
        vec![utils::push("-0.25", "0"), utils::push("0.25", "0")]
    );
}

/// Unyielding idle bodies keep no spacing: touching, neither drifts.
#[test]
fn unyielding_idle_pair_never_drifts() {
    let pushes = utils::pushes(
        &[utils::resting("5", "5.5"), utils::resting("6", "5.5")],
        |_, _| Meeting::Unyielding,
    );

    assert_eq!(pushes, vec![FixedVec2::ZERO, FixedVec2::ZERO]);
}

//
// ─── Masks and bounds ─────────────────────────────────────────────────────────
//

/// Bodies occupying disjoint layers pass through each other.
#[test]
fn disjoint_masks_pass_through() {
    let pushes = utils::pushes(
        &[utils::resting("2.5", "5.5"), utils::flying("2.7", "5.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

/// A dense stack sums pushes from every contact, each axis clamped to half a
/// cell.
#[test]
fn stacked_pushes_clamp_per_axis() {
    let pushes = utils::pushes(
        &[
            utils::walking("2.5", "5.5", "1", "0"),
            utils::walking("2.5", "5.5", "1", "0"),
            utils::walking("2.5", "5.5", "1", "0"),
        ],
        |_, _| Meeting::Yielding,
    );

    // Three walkers on one point: each pair overlaps by 1 and swerves, 0.5
    // radially each along x and a quarter of that, 0.125, sideways. The outer
    // bodies sum two pairs, 1 along x clamped to 0.5 and 2 × 0.125 = 0.25
    // sideways; the middle body's two pairs cancel exactly.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.5", "0.25"),
            utils::push("0", "0"),
            utils::push("0.5", "-0.25")
        ]
    );
}

//
// ─── Mixed footprints ─────────────────────────────────────────────────────────
//

/// Contact geometry runs between the bodies' positions, their middles: a small
/// body tucked against a wide one's far corner overlaps it even though their
/// footprints' low corners are far apart.
#[test]
fn small_body_overlapping_wide_body_far_side_is_pushed() {
    // Wide 2x2 with its middle at (6,6), radius 1. Small with its middle at
    // (7,7), radius 0.5 — distance √2 < reach 1.5, while their low corners,
    // (5,5) and (6.5,6.5), lie 2.12 apart and would read as clear.
    let wide = Body {
        bearing: Bearing::Working,
        ..utils::wide("6", "6", 2)
    };
    let pushes = utils::pushes(&[wide, utils::working("7", "7")], |_, _| Meeting::Yielding);

    // Both are working, so each takes half the overlap (1.5 − √2 ≈ 0.0858), split
    // onto the axes by the diagonal's 1/√2 — ≈0.03033 per axis, exact in
    // fixed-point bits. The shares are mirror images along the center line.
    let axis_share = FixedI64::from_bits(0x7c3_b666);
    assert_eq!(
        pushes[1],
        FixedVec2 {
            x: axis_share,
            y: axis_share,
        },
        "the small body must be shoved off the wide one's far corner"
    );
    assert_eq!(
        pushes[0],
        FixedVec2 {
            x: -axis_share,
            y: -axis_share,
        },
        "the wide body must take the equal opposite share"
    );
}

/// The mirror case: a small body near a wide one's low corner is farther from
/// its middle than the corners suggest, so no phantom push parts them.
#[test]
fn small_body_clear_of_wide_body_near_side_rests_untouched() {
    // Wide 2x2 with its middle at (6,6). Small with its middle at (4.5,4.5) —
    // distance 2.12 ≥ reach 1.5, while their low corners, (5,5) and (4,4),
    // lie 1.41 apart and would read as overlapping.
    let pushes = utils::pushes(
        &[utils::wide("6", "6", 2), utils::resting("4.5", "4.5")],
        |_, _| Meeting::Yielding,
    );

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

//
// ─── Giving way ───────────────────────────────────────────────────────────────
//

/// A lower rank gives way to a higher, a higher never to a lower: a free body
/// to a working, planted or fixed one, a working body to a planted one, a
/// planted body to a fixed one.
#[test]
fn lower_rank_gives_way_to_higher() {
    for meeting in [Meeting::Yielding, Meeting::Unyielding] {
        assert!(contact::gives_way(Bearing::Idle, Bearing::Working, meeting));
        assert!(contact::gives_way(
            Bearing::Working,
            Bearing::Planted,
            meeting
        ));
        assert!(!contact::gives_way(
            Bearing::Planted,
            Bearing::Working,
            meeting
        ));
        assert!(contact::gives_way(Bearing::Inert, Bearing::Fixed, meeting));
        assert!(contact::gives_way(
            Bearing::Planted,
            Bearing::Fixed,
            meeting
        ));
        assert!(!contact::gives_way(
            Bearing::Working,
            Bearing::Idle,
            meeting
        ));
        assert!(!contact::gives_way(
            Bearing::Fixed,
            Bearing::Planted,
            meeting
        ));
    }
}

/// Between equal ranks below fixed, a walker always gives way, a working,
/// planted or idle body only to a body it meets yielding; two fixed bodies
/// never.
#[test]
fn equal_ranks_give_way_by_meeting_unless_walking() {
    let walking = Bearing::Walking(utils::heading("1", "0"));
    assert!(contact::gives_way(
        walking,
        Bearing::Idle,
        Meeting::Unyielding
    ));
    assert!(contact::gives_way(
        Bearing::Idle,
        Bearing::Idle,
        Meeting::Yielding
    ));
    assert!(!contact::gives_way(
        Bearing::Idle,
        walking,
        Meeting::Unyielding
    ));
    assert!(contact::gives_way(
        Bearing::Planted,
        Bearing::Planted,
        Meeting::Yielding
    ));
    assert!(!contact::gives_way(
        Bearing::Working,
        Bearing::Working,
        Meeting::Unyielding
    ));
    assert!(!contact::gives_way(
        Bearing::Fixed,
        Bearing::Fixed,
        Meeting::Yielding
    ));
}
