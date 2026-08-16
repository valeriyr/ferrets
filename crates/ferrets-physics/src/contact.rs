//! Pairwise contact resolution: how touching bodies push each other apart.

use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};
use ferrets_pathfinder::layer_mask::LayerMask;

use crate::body::{self, Body};

/// The farthest one pass may displace a body, in cells per axis — high
/// enough that contact is rigid against full-speed movement, bounded so
/// dense piles cannot explode.
fn max_push() -> FixedI64 {
    FixedI64::from_num(0.5)
}

/// How far per pass two separated bodies sharing a cell creep apart.
fn cell_share_creep() -> FixedI64 {
    FixedI64::from_num(0.1)
}

/// One pass of pairwise separation: the displacement each body takes from
/// every contact it is in, clamped per axis to [`max_push`].
///
/// Overlapping bodies shove each other apart along their offset — half the
/// overlap each between equals, the whole of it to a body pressed by a
/// walker it is not pressing back. A contact that needs to pass (see [`needs_swerve`]) also
/// gets a clockwise sideways share — the traffic rule that keeps exactly
/// aligned meetings moving: lattice walking aligns positions bit for bit, so
/// dead-straight head-on contacts are the common case, and a purely radial
/// push would stall them nose to nose; each body swerving consistently to
/// its own side parts the pair deterministically. Every other contact parts
/// along the straight line between the bodies — under sustained contact, a
/// sideways share is a torque that would roll companions around each other
/// and mill a settled pile forever. A dead-on overlap breaks the symmetry
/// along the x axis by pair order.
///
/// Separated bodies whose centers share a cell drift apart the same way:
/// circles pack tighter than one per cell (two can rest a full diameter
/// apart inside one cell diagonally), but a cell claims as one boolean —
/// ten units on nine cells is uncountable occupancy. The creep continues
/// until every settled body stands on its own cell, so at rest occupancy is
/// exact: one body, one cell.
pub fn separations(bodies: &[Body]) -> Vec<FixedVec2> {
    let mut pushes = vec![FixedVec2::ZERO; bodies.len()];
    for first in 0..bodies.len() {
        for second in (first + 1)..bodies.len() {
            if bodies[first].mask & bodies[second].mask == LayerMask::EMPTY {
                continue;
            }
            // Geometry runs between the circles' centers, which sit half a
            // footprint size past each anchor — the same point the terrain
            // checks use. Between bodies of equal size the anchor offset and
            // the center offset coincide, but a wide body's center is deeper in, and
            // measuring anchors would miss real overlap on one side of it
            // while phantom-pushing on the other.
            let first_center = body::center(bodies[first].position, bodies[first].size);
            let second_center = body::center(bodies[second].position, bodies[second].size);
            let dx = second_center.x.to_num::<FixedI64>() - first_center.x.to_num::<FixedI64>();
            let dy = second_center.y.to_num::<FixedI64>() - first_center.y.to_num::<FixedI64>();
            let distance = first_center.distance(second_center).to_num::<FixedI64>();
            let reach = (bodies[first].radius + bodies[second].radius).to_num::<FixedI64>();
            let share_cell =
                body::anchor(bodies[first].position) == body::anchor(bodies[second].position);
            if distance >= reach && !share_cell {
                continue;
            }

            let overlap = reach - distance;
            let total = if overlap > FixedI64::ZERO {
                overlap
            } else {
                // Cell-sharing without body overlap: a steady creep apart.
                cell_share_creep() * FixedI64::from_num(2)
            };
            // Pressure yields to intent: a body pressed by a walker it is not
            // itself pressing takes three quarters of the separation, so the
            // walker mostly keeps its line and the contest ends instead of
            // trading equal shoves forever — the churn that pinned workers at
            // crowded drop-offs. Not the whole of it: a walker that loses
            // nothing to contact rams arriving crowds into piles packed too
            // tight for the rest-creep to relax back to one body per cell.
            // Mutual presses and settled piles split evenly, as before.
            let presses_second = presses_toward(&bodies[first], dx, dy);
            let presses_first = presses_toward(&bodies[second], -dx, -dy);
            let quarter = total / FixedI64::from_num(4);
            let (first_share, second_share) = match (presses_second, presses_first) {
                (true, false) => (quarter, total - quarter),
                (false, true) => (total - quarter, quarter),
                _ => (total / FixedI64::from_num(2), total / FixedI64::from_num(2)),
            };
            let swerve = needs_swerve(&bodies[first], &bodies[second], dx, dy);
            let lateral = |share: FixedI64| {
                if swerve {
                    share / FixedI64::from_num(4)
                } else {
                    FixedI64::ZERO
                }
            };
            let (first_lateral, second_lateral) = (lateral(first_share), lateral(second_share));
            let (unit_x, unit_y) = if distance > FixedI64::ZERO {
                (dx / distance, dy / distance)
            } else {
                (FixedI64::ONE, FixedI64::ZERO)
            };
            // Each side's swerve is the clockwise perpendicular of its own
            // outward direction, so the two swerves point apart.
            pushes[first].x -= unit_x * first_share + unit_y * first_lateral;
            pushes[first].y -= unit_y * first_share - unit_x * first_lateral;
            pushes[second].x += unit_x * second_share + unit_y * second_lateral;
            pushes[second].y += unit_y * second_share - unit_x * second_lateral;
        }
    }
    for push in &mut pushes {
        push.x = push.x.clamp(-max_push(), max_push());
        push.y = push.y.clamp(-max_push(), max_push());
    }
    pushes
}

/// Whether a contact plays the swerving traffic rule or parts radially.
///
/// Swerving is for contacts that must pass one another and would stall on
/// their own: bodies pressing into each other — a head-on meeting, a crowd
/// converging on one spot — and a walker pressing into a parked body. A
/// body presses when its heading has a component toward the other body.
/// Everything else parts radially, because a sideways share under sustained
/// contact is a torque: companions marching the same way press one-sidedly
/// at most (the swerve would roll them around each other all the way), and
/// a settled pile presses not at all (it would mill forever instead of
/// relaxing). `dx`/`dy` is the offset from `first` to `second`.
fn needs_swerve(first: &Body, second: &Body, dx: FixedI64, dy: FixedI64) -> bool {
    // Coincident bodies — opposing walks stepping onto the same point in
    // the same tick — give the press test no direction; any walker in the
    // stack must swerve out, or the pair re-converges every tick.
    if dx == FixedI64::ZERO && dy == FixedI64::ZERO {
        return first.heading.is_some() || second.heading.is_some();
    }
    let toward_second = presses_toward(first, dx, dy);
    let toward_first = presses_toward(second, -dx, -dy);
    match (first.heading, second.heading) {
        (None, None) => false,
        (Some(_), None) => toward_second,
        (None, Some(_)) => toward_first,
        (Some(_), Some(_)) => toward_second && toward_first,
    }
}

/// Whether `body`'s heading has a component along the offset `(dx, dy)` —
/// it is pressing toward whatever stands there.
fn presses_toward(body: &Body, dx: FixedI64, dy: FixedI64) -> bool {
    body.heading
        .is_some_and(|heading| heading.x * dx + heading.y * dy > FixedI64::ZERO)
}
