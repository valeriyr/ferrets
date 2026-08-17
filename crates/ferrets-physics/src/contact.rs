//! Pairwise contact resolution: how touching bodies push each other apart.

use std::cmp::Ordering;

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

/// How much harder a body pressing into a contact is to displace than the same
/// body at rest — the momentum a walker carries into whatever it meets. A body
/// needs more than this much of a weight advantage to hold its ground against
/// one walking into it.
///
/// Nothing bounds that advantage, deliberately. Past the threshold the contest
/// inverts and the walker becomes the one carried off, which is what lets a
/// heavy body read as a rock in the road rather than a weight that stops
/// meaning anything the moment it clears three times its neighbours. Capping
/// the pressing share at half would also split the rule in two, since bodies
/// meeting head-on or settled in a pile already contest on weight alone with no
/// ceiling. A walker is not left stalled nose-first against a heavy body
/// either: the sideways share rides on the radial one, so the heavier the thing
/// met, the harder the walker is turned aside rather than merely held.
fn press_stiffening() -> FixedI64 {
    FixedI64::from_num(3)
}

/// One pass of pairwise separation: the displacement each body takes from
/// every contact it is in, clamped per axis to [`max_push`].
///
/// Overlapping bodies shove each other apart along their offset, each carrying
/// the share of the overlap the other's resistance earns it — half each between
/// equals, less to the heavier of unequals, and less again to a body pressing
/// into the contact. A contact that needs to pass (see [`needs_swerve`]) also
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
            // Contact is a contest of resistance: each body takes the share of
            // the separation the other's resistance earns it, so equals part
            // evenly and a heavier body yields less than what it meets.
            // Pressing into a contact resists as three times the same body at
            // rest — the momentum a walker carries — so against a resting equal
            // a walker mostly keeps its line and the contest ends instead of
            // trading equal shoves forever, the churn that pinned workers at
            // crowded drop-offs. Not the whole of it: a walker that loses
            // nothing to contact rams arriving crowds into piles packed too
            // tight for the rest-creep to relax back to one body per cell. The
            // stiffening cancels wherever both bodies are in the same state, so
            // mutual presses and settled piles are pure weight contests.
            let presses_second = presses_toward(&bodies[first], dx, dy);
            let presses_first = presses_toward(&bodies[second], -dx, -dy);
            let first_stiffness = stiffness(&bodies[first], presses_second);
            let second_stiffness = stiffness(&bodies[second], presses_first);
            // The stiffer body's share is the measured one and the yielding body
            // takes what is left, so the pair parts by the whole separation and
            // the rounding lands on the side already giving the most ground.
            // Equals have no stiffer side to measure from and halve it instead.
            //
            // The two cases have to round differently, however tempting one rule
            // for both looks: measuring both shares everywhere, or taking one as
            // the remainder everywhere, moves a settled crowd by a single bit a
            // pass, and that is enough to leave it circling a small orbit
            // forever instead of coming to rest. Reading the comparison rather
            // than the pair position also keeps the split the same whichever
            // order the bodies are walked in.
            //
            // Only the unequal cases divide, and only they can: one side
            // outweighing the other puts the greater above nothing and the two
            // together above nothing with it. Equal resistances are the sole way
            // the contest comes to nothing at all — both bodies weightless — and
            // that case never reaches a division.
            let (first_share, second_share) = match first_stiffness.cmp(&second_stiffness) {
                Ordering::Equal => {
                    let half = total / FixedI64::from_num(2);
                    (half, half)
                }
                Ordering::Greater => {
                    let contested = first_stiffness.saturating_add(second_stiffness);
                    let first_share = total * second_stiffness / contested;
                    (first_share, total - first_share)
                }
                Ordering::Less => {
                    let contested = first_stiffness.saturating_add(second_stiffness);
                    let second_share = total * first_stiffness / contested;
                    (total - second_share, second_share)
                }
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

/// How strongly `body` resists this contact: what it weighs, stiffened while it
/// presses into the contact.
fn stiffness(body: &Body, presses: bool) -> FixedI64 {
    // Saturating, because nothing bounds an authored weight and this runs in
    // lockstep: wrapping arithmetic would turn an absurd one negative, which
    // both inverts the contest and breaks the one thing the split relies on —
    // that resistance is never below nothing. Saturating also keeps a debug
    // peer, which would otherwise panic, in step with a release one.
    let weight = body.weight.saturating_to_num::<FixedI64>();
    if presses {
        weight.saturating_mul(press_stiffening())
    } else {
        weight
    }
}

/// Whether `body`'s heading has a component along the offset `(dx, dy)` —
/// it is pressing toward whatever stands there.
fn presses_toward(body: &Body, dx: FixedI64, dy: FixedI64) -> bool {
    body.heading
        .is_some_and(|heading| heading.x * dx + heading.y * dy > FixedI64::ZERO)
}
