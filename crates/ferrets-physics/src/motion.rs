//! Moving one body toward an aim over its ground: the one way a body changes
//! place, whether it walks or is pushed.

use std::cmp::Ordering;

use ferrets_geometry::shape::Shape;
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};

use crate::ground::Ground;

/// At most how many times a walk is halved to find the farthest point along
/// it the ground admits: to a millionth of the walk.
const HALVINGS: u32 = 20;

/// How near two points lie and still count as one, in cells: 2⁻²⁴ — how deep
/// two circles may overlap and still touch, and how short a move is none.
pub(crate) const TOUCH_SLACK: FixedI64 = FixedI64::from_bits(1 << 8);

/// How many nudges — none, then a single bit, then each twice the last — may
/// carry a point rounding left inside an obstacle back out: to 2²⁶ bits, 1/64
/// cell.
const NUDGES: u32 = 28;

/// What a move toward an aim took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// The whole aim.
    Whole(FixedUVec2),
    /// A point turned from the aim: slid along what it met.
    Diverted(FixedUVec2),
    /// The point where the move first met an obstacle, short of the aim.
    Stopped(FixedUVec2),
    /// Nothing: the body stands where it was.
    Stood,
}

impl Advance {
    /// Where the body stands after the move, it having stood at `from`.
    pub fn point(self, from: FixedUVec2) -> FixedUVec2 {
        match self {
            Advance::Whole(point) | Advance::Diverted(point) | Advance::Stopped(point) => point,
            Advance::Stood => from,
        }
    }
}

/// Where the body of `ground` gets, moving toward `aim`: the first of these
/// the ground admits —
/// 1. the aim itself, when the walk there meets no obstacle circle on the
///    way;
/// 2. when the walk meets an obstacle circle, the walk slid along that
///    circle's tangent at the first touch by what the rest of the walk
///    carries that way, never less than a quarter of the rest, when the walk
///    there meets no other obstacle circle; else that point with one axis
///    dropped (the dominant axis kept first), when the walk there meets no
///    obstacle circle at all;
/// 3. the first touch of that circle, and it with one axis dropped, on the
///    same terms;
/// 4. the aim with one axis dropped, when the walk there meets no obstacle
///    circle;
/// 5. the farthest point along the walk the ground admits, short of the
///    first obstacle circle the walk meets — the first touch of the terrain
///    or the map's edge;
/// 6. standing.
///
/// A point of 2–5 within [`TOUCH_SLACK`] of the start is no move, unless the
/// aim itself lies that near.
///
/// The obstacle met is the one the walk reaches first, ties to the earlier
/// in the ground's list. A slide runs the way the walk leans past the
/// obstacle, clockwise when the walk is aimed at its middle exactly. No point
/// is farther from the body than the aim.
pub fn advance(aim: FixedUVec2, ground: &Ground) -> Advance {
    let position = ground.walker().middle();
    let walk = Walk::new(ground.walker(), aim);
    let met = first_met(&walk, ground.obstacles(), Among::All);
    if aim == position || met.is_none() && ground.admits(aim) {
        return Advance::Whole(aim);
    }
    let short_walk = !moves(position, aim);
    let is_move = |point: FixedUVec2| {
        if short_walk {
            point != position
        } else {
            moves(position, point)
        }
    };
    let moved = |point: FixedUVec2| is_move(point) && ground.admits(point);
    let walked_clear = |point: FixedUVec2, among: Among| {
        first_met(
            &Walk::new(ground.walker(), point),
            ground.obstacles(),
            among,
        )
        .is_none()
    };
    let dropped_clear = |point: FixedUVec2| moved(point) && walked_clear(point, Among::All);
    if let Some(touch) = met {
        // The slid point and the touch lie with the start on the outer side of
        // the tangent at the touch, so their walk stays off the circle met and
        // the circles identical to it; their axis drops are walked clear of
        // every circle.
        let around = |point: FixedUVec2| {
            if moved(point) && walked_clear(point, Among::Unlike(touch.obstacle)) {
                Some(point)
            } else {
                axis_drops(position, point)
                    .into_iter()
                    .find(|&point| dropped_clear(point))
            }
        };
        if let Some(point) = around(slide_past(&walk, touch)) {
            return Advance::Diverted(point);
        }
        if let Some(point) = around(touch.point) {
            return Advance::Stopped(point);
        }
    }
    if let Some(point) = axis_drops(position, aim)
        .into_iter()
        .find(|&point| dropped_clear(point))
    {
        return Advance::Diverted(point);
    }
    let limit = met.map_or(FixedI64::ONE, |touch| touch.fraction);
    let farthest = farthest_along(position, aim, limit, ground);
    if is_move(farthest) {
        Advance::Stopped(farthest)
    } else {
        Advance::Stood
    }
}

/// The sum of two circles' radii: how near their middles come when they
/// touch.
///
/// Panics if either is not a circle.
pub(crate) fn reach_of(walker: Shape, obstacle: Shape) -> FixedU64 {
    let radius = |shape: Shape| shape.radius().expect("contact is between circles");
    radius(walker) + radius(obstacle)
}

/// The unit direction of a signed offset, exact to a bit whatever its length
/// or direction, and its length as [`length`] gives it; `None` for a zero
/// offset.
pub(crate) fn unit((x, y): (FixedI64, FixedI64)) -> Option<((FixedI64, FixedI64), FixedI64)> {
    let (x, y) = (i128::from(x.to_bits()), i128::from(y.to_bits()));
    let larger = x.unsigned_abs().max(y.unsigned_abs());
    if larger == 0 {
        return None;
    }
    // Scaled up until the larger component has 62 bits, so the root of the
    // squares' sum keeps 61 bits or more; the squares' sum stays within 2¹²⁷.
    let shift = larger.leading_zeros().saturating_sub(128 - 62);
    let (x, y) = (x << shift, y << shift);
    let length = (x.unsigned_abs().pow(2) + y.unsigned_abs().pow(2)).isqrt();
    let length = i128::try_from(length).expect("a root of a sum up to 2¹²⁷ fits");
    let component = |value: i128| {
        FixedI64::from_bits(
            i64::try_from((value << FixedI64::FRAC_NBITS) / length)
                .expect("a unit's component lies within one"),
        )
    };
    let unscaled = FixedI64::from_bits(
        i64::try_from(length >> shift).expect("an offset's length fits the type"),
    );
    Some(((component(x), component(y)), unscaled))
}

/// The length of a signed offset.
pub(crate) fn length((x, y): (FixedI64, FixedI64)) -> FixedI64 {
    signed(
        FixedUVec2::new(
            x.unsigned_abs().to_num::<FixedU64>(),
            y.unsigned_abs().to_num::<FixedU64>(),
        )
        .length(),
    )
}

/// A coordinate as a signed value.
pub(crate) fn signed(value: FixedU64) -> FixedI64 {
    value.to_num::<FixedI64>()
}

/// A signed point held at the map's origin.
pub(crate) fn unsigned((x, y): (FixedI64, FixedI64)) -> FixedUVec2 {
    let clamp = |value: FixedI64| value.max(FixedI64::ZERO).to_num::<FixedU64>();
    FixedUVec2::new(clamp(x), clamp(y))
}

/// Whether a body standing at `from` moves by standing at `to`: the two lie
/// farther apart than [`TOUCH_SLACK`].
fn moves(from: FixedUVec2, to: FixedUVec2) -> bool {
    signed(from.distance(to)) > TOUCH_SLACK
}

/// The farthest point on the walk from `position` toward `aim`, short of the
/// fraction `limit` of it, the ground admits: the walk halved between a point
/// admitted and one refused until both are the same point, at most
/// [`HALVINGS`] times; `position` when none nearer than that is.
fn farthest_along(
    position: FixedUVec2,
    aim: FixedUVec2,
    limit: FixedI64,
    ground: &Ground,
) -> FixedUVec2 {
    let (step_x, step_y) = offset(position, aim);
    let at = |fraction: FixedI64| {
        unsigned((
            signed(position.x) + step_x * fraction,
            signed(position.y) + step_y * fraction,
        ))
    };
    let (mut admitted, mut refused) = (FixedI64::ZERO, limit);
    for _ in 0..HALVINGS {
        if at(admitted) == at(refused) {
            break;
        }
        let middle = (admitted + refused) / FixedI64::from_num(2);
        if ground.admits(at(middle)) {
            admitted = middle;
        } else {
            refused = middle;
        }
    }
    at(admitted)
}

/// `point` with each axis dropped toward `position`: the axis the move from
/// `position` covers most kept first, ties keeping x.
fn axis_drops(position: FixedUVec2, point: FixedUVec2) -> [FixedUVec2; 2] {
    let along_x = FixedUVec2::new(point.x, position.y);
    let along_y = FixedUVec2::new(position.x, point.y);
    if point.x.abs_diff(position.x) >= point.y.abs_diff(position.y) {
        [along_x, along_y]
    } else {
        [along_y, along_x]
    }
}

/// One body's walk: where it starts, and the step it takes.
struct Walk {
    /// The walker's circle where it starts.
    walker: Shape,
    /// The offset from its middle to where the walk aims.
    step: (FixedI64, FixedI64),
    /// The length of `step`.
    length: FixedI64,
    /// `step` scaled to length one; zero for a walk of no length.
    unit: (FixedI64, FixedI64),
}

impl Walk {
    /// The walk of `walker` toward `aim`.
    fn new(walker: Shape, aim: FixedUVec2) -> Self {
        let step = offset(walker.middle(), aim);
        let (unit, length) =
            unit(step).unwrap_or(((FixedI64::ZERO, FixedI64::ZERO), FixedI64::ZERO));
        Self {
            walker,
            step,
            length,
            unit,
        }
    }
}

/// Which of a ground's obstacles a walk is tested against.
#[derive(Debug, Clone, Copy)]
enum Among {
    /// Every one.
    All,
    /// Every one but those identical to this circle, which share its rim.
    Unlike(Shape),
}

/// Where a walk first touches an obstacle circle.
#[derive(Debug, Clone, Copy)]
struct Touch {
    /// The fraction of the walk taken at the touch.
    fraction: FixedI64,
    /// The walker's middle at the touch, backed off along the walk until it
    /// lies on or outside the obstacle's rim.
    point: FixedUVec2,
    /// The unit way out from the obstacle's middle to the touch.
    away: (FixedI64, FixedI64),
    /// The obstacle touched.
    obstacle: Shape,
    /// How near the walker's middle comes to the obstacle's at its rim: their
    /// radii's sum, or how near it already stands when the walk starts
    /// inside it.
    reach: FixedU64,
}

/// The first touch, along `walk`, of any of the `obstacles` it tests
/// (`among`): the smallest fraction, ties to the earlier obstacle.
fn first_met(walk: &Walk, obstacles: &[Shape], among: Among) -> Option<Touch> {
    obstacles
        .iter()
        .filter(|&&obstacle| match among {
            Among::All => true,
            Among::Unlike(skipped) => obstacle != skipped,
        })
        .filter_map(|&obstacle| touch(walk, obstacle))
        .fold(None, |best: Option<Touch>, touch| match best {
            Some(best) if best.fraction <= touch.fraction => Some(best),
            Some(_) | None => Some(touch),
        })
}

/// Where `walk` first touches `obstacle`, if the walk heads toward it and
/// reaches deeper than [`TOUCH_SLACK`] inside its rim; a walk that starts
/// inside it, heading toward its middle, meets it at once, its rim where the
/// walker stands. A walker on its middle leaves it whichever way it heads.
///
/// Panics if the walker or `obstacle` is not a circle.
fn touch(walk: &Walk, obstacle: Shape) -> Option<Touch> {
    let position = walk.walker.middle();
    let center = obstacle.middle();
    let (away_x, away_y) = offset(center, position);
    // In the walk's own frame: `along` is the middle's offset along the
    // walk's unit direction, `across` its distance off the walk's line.
    let along = away_x * walk.unit.0 + away_y * walk.unit.1;
    if walk.length == FixedI64::ZERO || along >= FixedI64::ZERO {
        return None;
    }
    let reach = reach_of(walk.walker, obstacle);
    let standing = position.distance(center);
    if standing < reach {
        return Some(Touch {
            fraction: FixedI64::ZERO,
            point: position,
            away: unit((away_x, away_y))
                .expect("a walk heading toward a middle starts off it")
                .0,
            obstacle,
            reach: standing,
        });
    }
    let across = away_x * walk.unit.1 - away_y * walk.unit.0;
    let reach_signed = signed(reach);
    // The walk's nearest point to the middle: where it passes closest, or its
    // end when it ends short of that.
    let nearest = if -along >= walk.length {
        length((away_x + walk.step.0, away_y + walk.step.1))
    } else {
        across.abs()
    };
    if nearest + TOUCH_SLACK >= reach_signed {
        return None;
    }
    // The walk touches where it comes `reach` from the middle, half a chord
    // of √(reach² − across²) short of the point nearest it.
    let half_chord = square_root(reach_signed * reach_signed - across * across);
    let to_touch = -along - half_chord;
    if to_touch > walk.length {
        return None;
    }
    let fraction = (to_touch / walk.length).max(FixedI64::ZERO);
    let (step_x, step_y) = walk.step;
    let at = (
        signed(position.x) + step_x * fraction,
        signed(position.y) + step_y * fraction,
    );
    let (away, _) = unit((at.0 - signed(center.x), at.1 - signed(center.y)))?;
    let back = (-walk.unit.0, -walk.unit.1);
    let point = outside(at, back, obstacle, reach)?;
    Some(Touch {
        fraction,
        point,
        away,
        obstacle,
        reach,
    })
}

/// `walk` slid along the obstacle's tangent at `touch` by what the rest of
/// the walk carries that way, never less than a quarter of the rest: the way
/// the walk leans past the obstacle, clockwise when it leans neither way;
/// nudged out onto the obstacle's rim, or the touch itself when it cannot
/// be.
fn slide_past(walk: &Walk, touch: Touch) -> FixedUVec2 {
    let (step_x, step_y) = walk.step;
    let away = touch.away;
    // The unit tangent, clockwise of the way out.
    let (tangent_x, tangent_y) = (away.1, -away.0);
    let left = FixedI64::ONE - touch.fraction;
    let along = step_x * tangent_x + step_y * tangent_y;
    let least = left * walk.length / 4;
    let carried = left * along;
    let slide = match along.cmp(&FixedI64::ZERO) {
        Ordering::Equal => least,
        Ordering::Greater => carried.max(least),
        Ordering::Less => carried.min(-least),
    };
    let from = (signed(touch.point.x), signed(touch.point.y));
    let slid = (from.0 + tangent_x * slide, from.1 + tangent_y * slide);
    outside(slid, away, touch.obstacle, touch.reach).unwrap_or(touch.point)
}

/// `point`, nudged along the unit direction `away` — a bit, then twice as
/// far each time — until it lies at least `reach` from the `obstacle`'s
/// middle, rounding having left it inside; `None` when the nudges do not get
/// it there.
fn outside(
    point: (FixedI64, FixedI64),
    away: (FixedI64, FixedI64),
    obstacle: Shape,
    reach: FixedU64,
) -> Option<FixedUVec2> {
    let center = obstacle.middle();
    let mut nudge = FixedI64::ZERO;
    for _ in 0..NUDGES {
        let reached = unsigned((point.0 + away.0 * nudge, point.1 + away.1 * nudge));
        if reached.distance(center) >= reach {
            return Some(reached);
        }
        nudge = if nudge == FixedI64::ZERO {
            FixedI64::DELTA
        } else {
            nudge * 2
        };
    }
    None
}

/// The offset from `from` to `to`, signed.
fn offset(from: FixedUVec2, to: FixedUVec2) -> (FixedI64, FixedI64) {
    (signed(to.x) - signed(from.x), signed(to.y) - signed(from.y))
}

/// The square root of a value that is not negative, rounded down to the
/// type's precision.
fn square_root(value: FixedI64) -> FixedI64 {
    let bits = u128::try_from(value.to_bits()).expect("the root of a value not below zero");
    FixedI64::from_bits(i64::try_from((bits << 32).isqrt()).expect("a root fits the type"))
}
