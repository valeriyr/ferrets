//! The pairwise contact rules: who swerves, who parts radially, who creeps
//! apart, and how hard anyone may be shoved.

mod utils;

use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};
use ferrets_physics::contact;

//
// ─── Swerve ───────────────────────────────────────────────────────────────────
//

/// Head-on walkers press into each other mutually — the lattice's common
/// case. Each swerves to its own side on top of the radial part:
///
///   A →   ← B      pushes: A ↙  B ↗
#[test]
fn head_on_pair_swerves_apart() {
    let pushes = contact::separations(&[
        utils::walking("2", "5", "1", "0"),
        utils::walking("2.5", "5", "-1", "0"),
    ]);

    // Overlap one half → a quarter cell radially each, an eighth of that
    // sideways, each to its own side.
    assert_eq!(
        pushes,
        vec![
            utils::push("-0.25", "0.0625"),
            utils::push("0.25", "-0.0625")
        ]
    );
}

/// A walker pressing into a parked body must flow around it, so the contact
/// swerves.
#[test]
fn walker_pressing_rester_shoves_it_aside() {
    let pushes = contact::separations(&[
        utils::walking("2", "5", "1", "0"),
        utils::resting("2.5", "5"),
    ]);

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.125", "0.03125"),
            utils::push("0.375", "-0.09375")
        ],
        "a parked body yields three quarters of the separation to the walker \
         pressing it, swerving aside; the walker mostly keeps its line"
    );
}

/// Coincident bodies — opposing walks stepping onto the same point in one
/// tick — get no direction from the press test, but any walker in the stack
/// must still swerve out or the pair re-converges forever.
#[test]
fn coincident_walkers_separate() {
    let pushes = contact::separations(&[
        utils::walking("2", "5", "1", "0"),
        utils::walking("2", "5", "-1", "0"),
    ]);

    // A full-body overlap parts along the x axis by pair order, swerving.
    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0.125"), utils::push("0.5", "-0.125")]
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
    let pushes = contact::separations(&[
        utils::walking("2", "5", "1", "0"),
        utils::walking("2.5", "5", "1", "0"),
    ]);

    assert_eq!(
        pushes,
        vec![utils::push("-0.125", "0"), utils::push("0.375", "0")],
        "same-way companions must not swerve around each other"
    );
}

/// A settled pile presses not at all: overlapping resters part along the
/// straight line between them, so the pile relaxes instead of milling.
#[test]
fn resting_pair_parts_radially() {
    let pushes = contact::separations(&[utils::resting("2", "5"), utils::resting("2.5", "5")]);

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
    let pushes = contact::separations(&[
        utils::weighing(utils::resting("2", "5"), "1"),
        utils::weighing(utils::resting("2.5", "5"), "3"),
    ]);

    // Three quarters of the half-cell overlap to the light body, a quarter to
    // the body weighing three times as much.
    assert_eq!(
        pushes,
        vec![utils::push("-0.375", "0"), utils::push("0.125", "0")]
    );
}

/// Pressing into a contact resists as three times the same body at rest, so a
/// walker keeps its line until what it meets outweighs it by more than that.
/// Far past it, the roles invert: the parked body stands and the walker is the
/// one shoved aside.
#[test]
fn walker_pressing_far_heavier_rester_takes_larger_share() {
    let pushes = contact::separations(&[
        utils::weighing(utils::walking("2", "5", "1", "0"), "1"),
        utils::weighing(utils::resting("2.5", "5"), "9"),
    ]);

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.375", "0.09375"),
            utils::push("0.125", "-0.03125")
        ],
        "nine times the walker's weight inverts the shares an equal pair takes"
    );
}

/// The other end: a walker heavy enough plows a light parked body aside and
/// barely gives ground itself.
#[test]
fn heavy_walker_plows_light_rester_aside() {
    let pushes = contact::separations(&[
        utils::weighing(utils::walking("2", "5", "1", "0"), "5"),
        utils::resting("2.5", "5"),
    ]);

    assert_eq!(
        pushes,
        vec![
            utils::push("-0.03125", "0.0078125"),
            utils::push("0.46875", "-0.1171875")
        ]
    );
}

/// The press stiffening cancels wherever both bodies are in the same state, so
/// a head-on meeting is a pure weight contest — the shares two resting bodies
/// of these weights take, with the head-on swerve on top.
#[test]
fn mutual_press_splits_by_weight_alone() {
    let pushes = contact::separations(&[
        utils::weighing(utils::walking("2", "5", "1", "0"), "1"),
        utils::weighing(utils::walking("2.5", "5", "-1", "0"), "3"),
    ]);

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
    let pushes = contact::separations(&[
        utils::weighing(utils::resting("2", "5"), "1"),
        utils::weighing(utils::resting("2.5", "5"), "2"),
    ]);

    assert_eq!(
        pushes[1].x - pushes[0].x,
        FixedI64::from_num(0.5),
        "the pair must part by the whole overlap however the split truncates"
    );
}

/// Equals part by mirror-image shoves, so which body a pass happens to walk
/// first cannot decide who gives ground.
#[test]
fn equal_weights_part_symmetrically_whichever_order() {
    let pushes = contact::separations(&[utils::resting("2", "5"), utils::resting("2.5", "5")]);
    let reversed = contact::separations(&[utils::resting("2.5", "5"), utils::resting("2", "5")]);

    assert_eq!(pushes[0], -pushes[1]);
    assert_eq!(pushes[0], reversed[1]);
    assert_eq!(pushes[1], reversed[0]);
}

/// A body debuffed to weightlessness has nothing to weigh against what it
/// meets, so it yields the whole separation and the other stands.
#[test]
fn weightless_body_yields_whole_separation() {
    let pushes = contact::separations(&[
        utils::weighing(utils::resting("2", "5"), "0"),
        utils::weighing(utils::resting("2.5", "5"), "1"),
    ]);

    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0"), utils::push("0", "0")]
    );
}

/// Bodies debuffed to weightlessness have nothing left to weigh against each
/// other, so they fall back on parting evenly.
#[test]
fn weightless_bodies_part_evenly() {
    let pushes = contact::separations(&[
        utils::weighing(utils::resting("2", "5"), "0"),
        utils::weighing(utils::resting("2.5", "5"), "0"),
    ]);

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
    let against_parked = |weight: &str| {
        contact::separations(&[
            utils::weighing(utils::walking("2", "5", "1", "0"), weight),
            utils::resting("2.5", "5"),
        ])
    };

    // One weight past the third saturates the stiffening; the heaviest a weight
    // can even be authored saturates the conversion into it as well. Both stand
    // at the same ceiling, so the shares stop moving instead of wrapping.
    let saturated = against_parked("715827883");
    assert_eq!(saturated, against_parked("4000000000"));

    // Outweighed by the whole range, the walker gives up a single bit of the
    // half-cell overlap and the parked body takes everything left, swerving
    // aside by its own quarter of it.
    let bit = FixedI64::from_bits(1);
    let parked = FixedI64::from_num(0.5) - bit;
    assert_eq!(
        saturated,
        vec![
            FixedVec2 {
                x: -bit,
                y: FixedI64::ZERO,
            },
            FixedVec2 {
                x: parked,
                y: -(parked / FixedI64::from_num(4)),
            },
        ]
    );
}

/// Pressing stiffens a body by what it weighs, so a weightless one presses
/// with nothing: it yields the whole separation walking into a parked body,
/// exactly as it would meeting one at rest.
#[test]
fn weightless_walker_yields_whole_separation() {
    let pushes = contact::separations(&[
        utils::weighing(utils::walking("2", "5", "1", "0"), "0"),
        utils::resting("2.5", "5"),
    ]);

    assert_eq!(
        pushes,
        vec![utils::push("-0.5", "0.125"), utils::push("0", "0")],
        "the walker keeps only the swerve its own share earns it"
    );
}

//
// ─── Countable occupancy ──────────────────────────────────────────────────────
//

/// Separated bodies whose centers share a cell creep apart — circles pack
/// tighter than one boolean claim per cell can count:
///
///   ┌─────┐
///   │A    │   both centers in one cell, bodies a full diameter apart
///   │    B│
///   └─────┘
#[test]
fn cell_sharing_resters_creep_apart() {
    let pushes =
        contact::separations(&[utils::resting("4.6", "4.6"), utils::resting("5.4", "5.4")]);

    // The diagonal direction runs through the deterministic square root, so
    // its exact bits have no clean literal; the relations are exact instead:
    // outward on both axes, perfectly diagonal, perfectly symmetric.
    assert!(pushes[0].x < FixedI64::ZERO);
    assert_eq!(
        pushes[0].x, pushes[0].y,
        "the creep runs along the diagonal"
    );
    assert_eq!(pushes[0].x, -pushes[1].x, "the creep is symmetric");
    assert_eq!(pushes[0].y, -pushes[1].y);
}

/// Separated bodies on distinct cells are settled: nothing touches them.
#[test]
fn separated_bodies_on_distinct_cells_rest_untouched() {
    let pushes = contact::separations(&[utils::resting("4.4", "5"), utils::resting("5.6", "5")]);

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

//
// ─── Masks and bounds ─────────────────────────────────────────────────────────
//

/// Bodies occupying disjoint layers pass through each other.
#[test]
fn disjoint_masks_pass_through() {
    let pushes = contact::separations(&[utils::resting("2", "5"), utils::flying("2.2", "5")]);

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

/// A dense stack sums pushes from every contact, but the committed shove is
/// clamped per axis so piles cannot explode.
#[test]
fn stacked_pushes_clamp_per_axis() {
    let pushes = contact::separations(&[
        utils::walking("2", "5", "1", "0"),
        utils::walking("2", "5", "1", "0"),
        utils::walking("2", "5", "1", "0"),
    ]);

    // The outer bodies sum a full cell of push each; the clamp caps the x
    // axis at half a cell, and the middle body's contacts cancel exactly.
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

/// Contact geometry runs between circle centers, which sit half a footprint
/// past each anchor: a small body tucked against a wide one's far corner
/// overlaps it even though their anchors are far apart.
#[test]
fn small_body_overlapping_wide_body_far_side_is_pushed() {
    // Wide 2x2 at anchor (5,5): center (6,6), radius 1. Small at anchor
    // (6.5,6.5): center (7,7), radius 0.5 — center distance √2 < reach 1.5,
    // while the anchor distance (2.12) would read as clear.
    let pushes = contact::separations(&[utils::wide("5", "5", 2), utils::resting("6.5", "6.5")]);

    // Both rest, so each takes half the overlap (1.5 − √2 ≈ 0.0858), split
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

/// The mirror case: a small body near a wide one's anchor corner is farther
/// from its center than the anchors suggest, so no phantom push parts them.
#[test]
fn small_body_clear_of_wide_body_near_side_rests_untouched() {
    // Wide 2x2 at anchor (5,5): center (6,6). Small at anchor (4,4): center
    // (4.5,4.5) — center distance 2.12 ≥ reach 1.5, while the anchor distance
    // (1.41) would read as overlapping.
    let pushes = contact::separations(&[utils::wide("5", "5", 2), utils::resting("4", "4")]);

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}
