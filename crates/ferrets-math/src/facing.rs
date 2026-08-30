//! A direction as a whole-circle angle in binary units.

use crate::{FixedI64, FixedU64, fixed_vec2::FixedVec2};

/// Angle units in a full circle. A power of two, so the whole circle closes
/// exactly under [`u16`] wrapping and every direction has a value.
pub const PER_TURN: u32 = u16::MAX as u32 + 1;

/// Angle units in a quarter circle.
const PER_QUARTER: u32 = PER_TURN / 4;

/// Angle units in an eighth of a circle.
const PER_EIGHTH: u32 = PER_TURN / 8;

/// Angle units in one radian, as the constant an arc-tangent in radians is
/// scaled by to land in these units.
const PER_RADIAN: FixedU64 = FixedU64::lit("10430.37835047");

/// A direction, measured clockwise from north in units of a whole circle
/// ([`PER_TURN`] to the turn, so one unit is about a two-hundredth of a degree).
///
/// North is `-y` and east is `+x`: simulation `y` grows downward, so a clockwise
/// angle grows the way a compass bearing does. Every direction is one value —
/// two bodies looking the same way hold the same bits — and the circle closing
/// exactly means turning past north wraps rather than saturating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Facing(u16);

impl Facing {
    /// Toward `-y`, the zero bearing.
    pub const NORTH: Self = Self(0);
    /// Toward `+x`.
    pub const EAST: Self = Self((PER_QUARTER) as u16);
    /// Toward `+y`.
    pub const SOUTH: Self = Self((PER_QUARTER * 2) as u16);
    /// Toward `-x`.
    pub const WEST: Self = Self((PER_QUARTER * 3) as u16);

    /// The bearing of `delta`, or `None` when it points nowhere.
    ///
    /// Only the direction of `delta` is read, never its length, so a crawl and a
    /// stride down the same line give the same bearing.
    pub fn of(delta: FixedVec2) -> Option<Self> {
        if delta == FixedVec2::ZERO {
            return None;
        }
        // Northward, so the quarter angle is measured off the axis the bearing
        // starts from.
        let up = -delta.y;
        let across = magnitude(delta.x);
        let along = magnitude(up);
        let quarter = quarter_angle(across, along) as u16;
        // The quarter angle is the same in each of the four quarters; only where
        // it is measured from differs.
        let bits = match (delta.x >= FixedI64::ZERO, up >= FixedI64::ZERO) {
            (true, true) => quarter,
            (true, false) => (PER_QUARTER * 2) as u16 - quarter,
            (false, false) => (PER_QUARTER * 2) as u16 + quarter,
            (false, true) => 0u16.wrapping_sub(quarter),
        };
        Some(Self(bits))
    }

    /// The bearing this many units clockwise from north.
    #[inline]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// This bearing in units clockwise from north.
    #[inline]
    pub const fn to_bits(self) -> u16 {
        self.0
    }

    /// The turn from this bearing to `other`, the short way round: positive
    /// clockwise, negative anticlockwise, and never more than half a circle
    /// either way.
    #[inline]
    pub const fn difference(self, other: Self) -> i32 {
        other.0.wrapping_sub(self.0) as i16 as i32
    }

    /// How far this bearing is from `other`, whichever way round is shorter.
    #[inline]
    pub const fn distance(self, other: Self) -> u32 {
        self.difference(other).unsigned_abs()
    }

    /// This bearing turned toward `target` by at most `most` units, the short way
    /// round; `target` itself once it is within reach.
    pub fn turn_toward(self, target: Self, most: u32) -> Self {
        let apart = self.difference(target);
        // Half a circle is the longest turn there is, so any larger allowance
        // means the whole of it — a clamp one unit lower would leave a turn from
        // dead astern one unit short of completing.
        let most = most.min(PER_TURN / 2);
        if apart.unsigned_abs() <= most {
            return target;
        }
        let step = if apart > 0 {
            most as i32
        } else {
            -(most as i32)
        };
        Self(self.0.wrapping_add(step as i16 as u16))
    }
}

/// `degrees` as a count of angle units, for rates and spans authored in degrees.
pub fn units_of_degrees(degrees: FixedU64) -> u32 {
    let whole = FixedU64::from_num(PER_TURN);
    (degrees / FixedU64::from_num(360) * whole).to_num()
}

/// A component's distance from zero, as an unsigned scalar.
fn magnitude(component: FixedI64) -> FixedU64 {
    FixedU64::from_bits(component.unsigned_abs().to_bits())
}

/// The angle between a direction and the axis `along` lies on, in units, from
/// zero (straight along it) to a quarter circle (straight across it).
fn quarter_angle(across: FixedU64, along: FixedU64) -> u32 {
    if across == FixedU64::ZERO {
        return 0;
    }
    if along == FixedU64::ZERO {
        return PER_QUARTER;
    }
    // Measured from whichever axis is nearer, so the ratio the approximation
    // takes is never above one — where it is least accurate.
    if across <= along {
        arc_tangent(across / along)
    } else {
        PER_QUARTER - arc_tangent(along / across)
    }
}

/// The arc-tangent of `ratio`, in angle units, for a ratio of no more than one.
///
/// A polynomial rather than a table: it is exact at both ends — nothing is more
/// visible than a cardinal or diagonal heading being a unit off — and within a
/// tenth of a degree between them, which is a fortieth of the smallest angle any
/// rule here compares.
fn arc_tangent(ratio: FixedU64) -> u32 {
    const NEAR: FixedU64 = FixedU64::lit("0.2447");
    const FAR: FixedU64 = FixedU64::lit("0.0663");

    let eighth = FixedU64::from_num(PER_EIGHTH);
    let bend = (NEAR + FAR * ratio) * PER_RADIAN;
    (eighth * ratio + ratio * (FixedU64::ONE - ratio) * bend).to_num()
}
