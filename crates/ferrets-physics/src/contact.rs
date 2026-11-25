//! The rules of one pair of bodies in contact.

use std::cmp::Ordering;

use ferrets_math::{FixedI64, fixed_vec2::FixedVec2};

use crate::{
    body::{Bearing, Body, Heading, Rank},
    motion,
};

/// The gap two idle bodies keep between their circles, in cells.
const IDLE_SPACING: FixedI64 = FixedI64::lit("0.25");

/// The farthest one pass drifts two idle bodies apart, together, toward
/// [`IDLE_SPACING`].
pub(crate) const IDLE_DRIFT: FixedI64 = FixedI64::lit("0.25");

/// How far short of [`IDLE_SPACING`] two idle bodies may stand and still
/// count as spaced.
pub(crate) const IDLE_SLACK: FixedI64 = FixedI64::lit("0.015625");

/// The sine of the widest angle off a walker's heading at which a body it
/// presses is nudged aside: 30°.
const ASIDE_SINE: FixedI64 = FixedI64::lit("0.5");

/// How much harder a body pressing into a contact is to displace than the same
/// body at rest — the momentum a walker carries into whatever it meets. A body
/// needs more than this much of a weight advantage to hold its ground against
/// one walking into it; past it the walker is the one carried off.
const PRESS_STIFFENING: FixedI64 = FixedI64::const_from_int(3);

/// How two bodies in contact treat each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meeting {
    /// They shove each other by weight, and two idle ones keep the idle
    /// spacing.
    Yielding,
    /// Neither shoves the other by weight: an idle or inert body is a wall to
    /// a walker, which takes the whole separation; two walkers, two idle or
    /// inert bodies, or two of one rank above them halve it. No spacing.
    Unyielding,
}

/// Whether a body of bearing `body` gives way to one of bearing `to` they
/// meet as `meeting` says: its rank is below; or the ranks are equal, below
/// fixed, and it walks or the meeting is yielding.
pub fn gives_way(body: Bearing, to: Bearing, meeting: Meeting) -> bool {
    match body.rank().cmp(&to.rank()) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match body {
            Bearing::Fixed => false,
            Bearing::Walking(_) => true,
            Bearing::Working | Bearing::Planted | Bearing::Inert | Bearing::Idle => match meeting {
                Meeting::Yielding => true,
                Meeting::Unyielding => false,
            },
        },
    }
}

/// The gap `first` and `second` keep between their circles: the idle
/// spacing between two idle bodies that meet yielding, none otherwise.
pub(crate) fn spacing(meeting: Meeting, first: &Body, second: &Body) -> FixedI64 {
    match (meeting, first.bearing, second.bearing) {
        (Meeting::Yielding, Bearing::Idle, Bearing::Idle) => IDLE_SPACING,
        (
            Meeting::Yielding,
            Bearing::Walking(_)
            | Bearing::Working
            | Bearing::Planted
            | Bearing::Fixed
            | Bearing::Inert,
            _,
        )
        | (
            Meeting::Yielding,
            Bearing::Idle,
            Bearing::Walking(_)
            | Bearing::Working
            | Bearing::Planted
            | Bearing::Fixed
            | Bearing::Inert,
        )
        | (Meeting::Unyielding, _, _) => FixedI64::ZERO,
    }
}

/// The shares of `total` that `first` and `second` take, `(dx, dy)` the
/// offset from `first` to `second`: the lower rank takes the whole; two fixed
/// bodies take nothing; two of another rank part as `meeting` says — by
/// weight when yielding, a body pressing into the contact resisting
/// [`PRESS_STIFFENING`] times as hard; when unyielding a walker against an
/// idle or inert body takes the whole, and two walkers, two idle or inert
/// bodies, or two of one rank above them halve it.
pub(crate) fn shares(
    meeting: Meeting,
    total: FixedI64,
    first: &Body,
    second: &Body,
    dx: FixedI64,
    dy: FixedI64,
) -> (FixedI64, FixedI64) {
    match first.bearing.rank().cmp(&second.bearing.rank()) {
        Ordering::Greater => (FixedI64::ZERO, total),
        Ordering::Less => (total, FixedI64::ZERO),
        Ordering::Equal => match (first.bearing.rank(), meeting) {
            (Rank::Fixed, Meeting::Yielding | Meeting::Unyielding) => {
                (FixedI64::ZERO, FixedI64::ZERO)
            }
            (Rank::Planted | Rank::Working | Rank::Free, Meeting::Yielding) => contest(
                total,
                stiffness(first, presses_toward(first, dx, dy)),
                stiffness(second, presses_toward(second, -dx, -dy)),
            ),
            (Rank::Planted | Rank::Working | Rank::Free, Meeting::Unyielding) => {
                match (first.bearing, second.bearing) {
                    (Bearing::Walking(_), Bearing::Inert | Bearing::Idle) => {
                        (total, FixedI64::ZERO)
                    }
                    (Bearing::Inert | Bearing::Idle, Bearing::Walking(_)) => {
                        (FixedI64::ZERO, total)
                    }
                    (Bearing::Walking(_), Bearing::Walking(_))
                    | (Bearing::Inert | Bearing::Idle, Bearing::Inert | Bearing::Idle)
                    | (Bearing::Working, Bearing::Working)
                    | (Bearing::Planted, Bearing::Planted) => {
                        let half = total / FixedI64::from_num(2);
                        (half, half)
                    }
                    (Bearing::Fixed, _)
                    | (_, Bearing::Fixed)
                    | (
                        Bearing::Walking(_) | Bearing::Inert | Bearing::Idle,
                        Bearing::Working | Bearing::Planted,
                    )
                    | (
                        Bearing::Working | Bearing::Planted,
                        Bearing::Walking(_) | Bearing::Inert | Bearing::Idle,
                    )
                    | (Bearing::Working, Bearing::Planted)
                    | (Bearing::Planted, Bearing::Working) => unreachable!(
                        "unyielding shares are asked of two bodies of one rank below fixed"
                    ),
                }
            }
        },
    }
}

/// Whether two walkers pressing into each other must swerve past each other:
/// each heads toward a point beyond the other body, `(dx, dy)` the offset
/// from `first` to `second`. Two bodies stepping onto one point are pressed
/// apart by any walker among them.
pub(crate) fn swerves(first: &Body, second: &Body, dx: FixedI64, dy: FixedI64) -> bool {
    let coincident = dx == FixedI64::ZERO && dy == FixedI64::ZERO;
    match (first.bearing, second.bearing) {
        (
            Bearing::Walking(Heading::Toward(first_heading)),
            Bearing::Walking(Heading::Toward(second_heading)),
        ) => {
            let squared = dx * dx + dy * dy;
            coincident
                || first_heading.x * dx + first_heading.y * dy > squared
                    && -(second_heading.x * dx + second_heading.y * dy) > squared
        }
        (Bearing::Walking(Heading::Unaimed), Bearing::Walking(_))
        | (Bearing::Walking(_), Bearing::Walking(Heading::Unaimed))
        | (
            Bearing::Walking(_),
            Bearing::Working | Bearing::Planted | Bearing::Fixed | Bearing::Inert | Bearing::Idle,
        )
        | (
            Bearing::Working | Bearing::Planted | Bearing::Fixed | Bearing::Inert | Bearing::Idle,
            Bearing::Walking(_),
        ) => coincident,
        (
            Bearing::Working | Bearing::Planted | Bearing::Fixed | Bearing::Inert | Bearing::Idle,
            Bearing::Working | Bearing::Planted | Bearing::Fixed | Bearing::Inert | Bearing::Idle,
        ) => false,
    }
}

/// The sideways share `body` takes with `share`, taken back from a body at
/// offset `(dx, dy)` from it, `distance` away, positive clockwise of its way
/// out: the part of its heading across the line between them, never less
/// than a quarter of `share`, the side its heading points — clockwise when it
/// points along the line exactly; none when it does not walk or aims
/// nowhere.
pub(crate) fn lean(
    body: &Body,
    share: FixedI64,
    dx: FixedI64,
    dy: FixedI64,
    distance: FixedI64,
) -> FixedI64 {
    match body.bearing {
        Bearing::Walking(Heading::Toward(heading)) => {
            // The way out from the other body is (−dx, −dy); its clockwise
            // perpendicular is (−dy, dx).
            let across = heading.y * dx - heading.x * dy;
            let least = share / FixedI64::from_num(4);
            let sideways = share * sine(heading, across, distance);
            let sideways = sideways.max(least);
            match across.cmp(&FixedI64::ZERO) {
                Ordering::Less => -sideways,
                Ordering::Equal | Ordering::Greater => sideways,
            }
        }
        Bearing::Walking(Heading::Unaimed)
        | Bearing::Working
        | Bearing::Planted
        | Bearing::Fixed
        | Bearing::Inert
        | Bearing::Idle => FixedI64::ZERO,
    }
}

/// The sideways share an idle or inert `body` pressed by `walker` takes
/// with `share`, the walker at offset `(dx, dy)` from it, `distance` away,
/// positive clockwise of its way out: toward the side it lies off the
/// walker's heading, clockwise when it lies on it exactly, a quarter of
/// `share` dead-on, tapering to none as the heading's angle off it reaches
/// the one whose sine is [`ASIDE_SINE`]; none beyond.
pub(crate) fn aside(
    body: &Body,
    walker: &Body,
    share: FixedI64,
    dx: FixedI64,
    dy: FixedI64,
    distance: FixedI64,
) -> FixedI64 {
    let heading = match (body.bearing, walker.bearing) {
        (Bearing::Idle | Bearing::Inert, Bearing::Walking(Heading::Toward(heading))) => heading,
        (
            Bearing::Idle | Bearing::Inert,
            Bearing::Walking(Heading::Unaimed)
            | Bearing::Working
            | Bearing::Planted
            | Bearing::Fixed
            | Bearing::Inert
            | Bearing::Idle,
        )
        | (Bearing::Walking(_) | Bearing::Working | Bearing::Planted | Bearing::Fixed, _) => {
            return FixedI64::ZERO;
        }
    };
    // From the walker to this body is (−dx, −dy), this body's way out; the
    // heading pressing toward it has a positive part along that way.
    let along = -(heading.x * dx + heading.y * dy);
    if along <= FixedI64::ZERO {
        return FixedI64::ZERO;
    }
    // The way out's clockwise perpendicular is (−dy, dx); the heading's part
    // across it says which side of the walker's line this body lies.
    let across = heading.y * dx - heading.x * dy;
    let sine = sine(heading, across, distance);
    if sine >= ASIDE_SINE {
        return FixedI64::ZERO;
    }
    let nudge = share / FixedI64::from_num(4) * (FixedI64::ONE - sine / ASIDE_SINE);
    match across.cmp(&FixedI64::ZERO) {
        Ordering::Greater => -nudge,
        Ordering::Equal | Ordering::Less => nudge,
    }
}

/// Whether `body`'s heading has a component along the offset `(dx, dy)` —
/// it is pressing toward whatever stands there.
fn presses_toward(body: &Body, dx: FixedI64, dy: FixedI64) -> bool {
    match body.bearing {
        Bearing::Walking(Heading::Toward(heading)) => {
            heading.x * dx + heading.y * dy > FixedI64::ZERO
        }
        Bearing::Walking(Heading::Unaimed)
        | Bearing::Working
        | Bearing::Planted
        | Bearing::Fixed
        | Bearing::Inert
        | Bearing::Idle => false,
    }
}

/// How strongly `body` resists this contact: what it weighs, stiffened while it
/// presses into the contact, saturating at the type's bound.
fn stiffness(body: &Body, presses: bool) -> FixedI64 {
    let weight = body.weight.saturating_to_num::<FixedI64>();
    if presses {
        weight.saturating_mul(PRESS_STIFFENING)
    } else {
        weight
    }
}

/// The shares of `total` two contesting bodies take, each the share the
/// other's resistance earns it, the rounding left to the less stiff.
fn contest(
    total: FixedI64,
    first_stiffness: FixedI64,
    second_stiffness: FixedI64,
) -> (FixedI64, FixedI64) {
    // The stiffer body's share is measured and the other takes what is left;
    // equals halve. The ratio is never above one and is taken first.
    match first_stiffness.cmp(&second_stiffness) {
        Ordering::Equal => {
            let half = total / FixedI64::from_num(2);
            (half, half)
        }
        Ordering::Greater => {
            let contested = first_stiffness.saturating_add(second_stiffness);
            let first_share = total * (second_stiffness / contested);
            (first_share, total - first_share)
        }
        Ordering::Less => {
            let contested = first_stiffness.saturating_add(second_stiffness);
            let second_share = total * (first_stiffness / contested);
            (total - second_share, second_share)
        }
    }
}

/// The sine of the angle between `heading` and a line `distance` long whose
/// cross product with it is `across`, held to one: zero for a heading or a
/// line of no length.
fn sine(heading: FixedVec2, across: FixedI64, distance: FixedI64) -> FixedI64 {
    let heading_length = motion::length((heading.x, heading.y));
    if heading_length == FixedI64::ZERO || distance == FixedI64::ZERO {
        return FixedI64::ZERO;
    }
    (across.abs() / heading_length).min(distance) / distance
}
