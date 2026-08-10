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
        utils::walking(2.0, 5.0, 1.0, 0.0),
        utils::walking(2.5, 5.0, -1.0, 0.0),
    ]);

    // Overlap one half → a quarter cell radially each, an eighth of that
    // sideways, each to its own side.
    assert_eq!(
        pushes,
        vec![utils::push(-0.25, 0.0625), utils::push(0.25, -0.0625)]
    );
}

/// A walker pressing into a parked body must flow around it, so the contact
/// swerves.
#[test]
fn walker_pressing_rester_shoves_it_aside() {
    let pushes =
        contact::separations(&[utils::walking(2.0, 5.0, 1.0, 0.0), utils::resting(2.5, 5.0)]);

    assert_eq!(
        pushes,
        vec![utils::push(-0.125, 0.03125), utils::push(0.375, -0.09375)],
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
        utils::walking(2.0, 5.0, 1.0, 0.0),
        utils::walking(2.0, 5.0, -1.0, 0.0),
    ]);

    // A full-body overlap parts along the x axis by pair order, swerving.
    assert_eq!(
        pushes,
        vec![utils::push(-0.5, 0.125), utils::push(0.5, -0.125)]
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
        utils::walking(2.0, 5.0, 1.0, 0.0),
        utils::walking(2.5, 5.0, 1.0, 0.0),
    ]);

    assert_eq!(
        pushes,
        vec![utils::push(-0.125, 0.0), utils::push(0.375, 0.0)],
        "same-way companions must not swerve around each other"
    );
}

/// A settled pile presses not at all: overlapping resters part along the
/// straight line between them, so the pile relaxes instead of milling.
#[test]
fn resting_pair_parts_radially() {
    let pushes = contact::separations(&[utils::resting(2.0, 5.0), utils::resting(2.5, 5.0)]);

    assert_eq!(
        pushes,
        vec![utils::push(-0.25, 0.0), utils::push(0.25, 0.0)]
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
    let pushes = contact::separations(&[utils::resting(4.6, 4.6), utils::resting(5.4, 5.4)]);

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
    let pushes = contact::separations(&[utils::resting(4.4, 5.0), utils::resting(5.6, 5.0)]);

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

//
// ─── Masks and bounds ─────────────────────────────────────────────────────────
//

/// Bodies occupying disjoint layers pass through each other.
#[test]
fn disjoint_masks_pass_through() {
    let pushes = contact::separations(&[utils::resting(2.0, 5.0), utils::flying(2.2, 5.0)]);

    assert_eq!(pushes[0], FixedVec2::ZERO);
    assert_eq!(pushes[1], FixedVec2::ZERO);
}

/// A dense stack sums pushes from every contact, but the committed shove is
/// clamped per axis so piles cannot explode.
#[test]
fn stacked_pushes_clamp_per_axis() {
    let pushes = contact::separations(&[
        utils::walking(2.0, 5.0, 1.0, 0.0),
        utils::walking(2.0, 5.0, 1.0, 0.0),
        utils::walking(2.0, 5.0, 1.0, 0.0),
    ]);

    // The outer bodies sum a full cell of push each; the clamp caps the x
    // axis at half a cell, and the middle body's contacts cancel exactly.
    assert_eq!(
        pushes,
        vec![
            utils::push(-0.5, 0.25),
            utils::push(0.0, 0.0),
            utils::push(0.5, -0.25)
        ]
    );
}
