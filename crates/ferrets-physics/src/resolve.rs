//! The contact pass: every body's displacement from the bodies it overlaps or
//! stands too near, judged from one gathering and applied once.

use ferrets_geometry::{location::Location, shape::Shape};
use ferrets_math::{FixedI64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_nav_grid::{layer_mask::LayerMask, nav_grid::NavGrid};

use crate::{
    body::{Body, Rank},
    contact::{self, IDLE_DRIFT, IDLE_SLACK, Meeting},
    ground::Ground,
    motion::{self, Advance, TOUCH_SLACK},
};

/// The farthest one pass may displace a body, in cells per axis.
const MAX_PUSH: FixedI64 = FixedI64::lit("0.5");

/// How far a body is tried the way a push takes it, in cells: the sliver step
/// its way is read from.
const ROOM: FixedI64 = FixedI64::lit("0.015625");

/// The step a way's chain part is rounded up to, a fraction of a sliver
/// step.
const WAY_STEP: FixedI64 = FixedI64::lit("0.015625");

/// How opposed two pushes on a body must be to squeeze it: the cosine of
/// their angle below minus this, more than 120° apart.
const SQUEEZE_OPPOSITION: FixedI64 = FixedI64::lit("0.5");

/// How far apart beyond touching two bodies still count as near, in cells:
/// more than two pushes can close between them in a pass, each held to
/// [`MAX_PUSH`] per axis — 2 × 0.5 × √2 ≈ 1.41.
const NEAR: FixedI64 = FixedI64::lit("1.5");

/// Where each of `bodies` stands after one pass of contact, in the order
/// given; `meeting(first, second)` says how the bodies at those indices meet,
/// first the lower; `grid` is the ground they stand on.
///
/// Each body near touching another, or drifting from it, has a *way* toward
/// where the other would push it: how much of a sliver step that way it makes
/// along the line, moved over the terrain, the map's edge and its walls — the
/// bodies that do not give way to it, and those within [`ROOM`] of touching
/// shut toward it — and no more than the bodies it would press on make of
/// theirs, rounded up to a [`WAY_STEP`] — the walls judged first, sweep after
/// sweep until no way newly shuts, then the chains, never wider than the
/// last sweep's, until nothing changes. Each overlapping pair's shares follow
/// rank, then the meeting, then weight ([`contact::shares`]). A body makes
/// along the line only what its way makes of its share — a shut one nothing
/// — and what it cannot goes to its partner if the partner gives way
/// and is not shut itself, the partner making of it what its own way makes,
/// leaning past by that ([`contact::lean`]); what neither makes stays. A body
/// is moved by its share held to what the bodies ahead allow where they are the
/// tighter limit, its move over the ground making the rest of the way. Two
/// walkers that must pass each other swerve apart by a quarter of what each
/// makes, clockwise ([`contact::swerves`]), and an idle or inert body pressed
/// by a walker heading nearly at it, shut or not, is nudged aside on its whole
/// share ([`contact::aside`]). Two idle bodies short of the idle spacing drift
/// apart by at most [`IDLE_DRIFT`] a pass, never handed on: a body is moved by
/// its drift as by its share, held to its way, none when pushed from roughly
/// opposite sides ([`squeezed`]), and a drift is taken whole after the
/// overlaps' move or not at all. Each body's overlap push, held to [`MAX_PUSH`]
/// per axis, is moved once by [`motion::advance`] over the ground its way is
/// judged on; a fixed body never moves. With nothing to part anywhere among
/// `bodies`, nothing is judged and nobody moves.
pub fn settle(
    bodies: &[Body],
    meeting: impl Fn(usize, usize) -> Meeting,
    grid: &NavGrid,
) -> Vec<Location> {
    let pairs = near_pairs(bodies, meeting);
    if pairs.iter().all(|pair| pair.parting(bodies).is_none()) {
        return bodies.iter().map(|body| body.location).collect();
    }
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); bodies.len()];
    for (index, pair) in pairs.iter().enumerate() {
        near[pair.first].push(index);
        near[pair.second].push(index);
    }
    let ways = ways(bodies, &pairs, &near, grid);
    let pushes = pushes(bodies, &pairs, &ways);
    bodies
        .iter()
        .zip(pushes)
        .enumerate()
        .map(|(index, (body, Push { overlap, drift }))| {
            match body.bearing.rank() {
                Rank::Fixed => return body.location,
                Rank::Free | Rank::Working | Rank::Planted => {}
            }
            if overlap == FixedVec2::ZERO && drift == FixedVec2::ZERO {
                return body.location;
            }
            let obstacles = walls(bodies, &pairs, &near[index], &ways, index);
            let position = body.location.middle();
            let ground = Ground::new(grid, body.mover, body.shape(), &obstacles);
            let pushed = motion::advance(displaced(position, overlap), &ground).point(position);
            let point = if drift == FixedVec2::ZERO {
                pushed
            } else {
                let from = Shape::Circle {
                    middle: pushed,
                    radius: body.radius,
                };
                let ground = Ground::new(grid, body.mover, from, &obstacles);
                match motion::advance(displaced(pushed, drift), &ground) {
                    Advance::Whole(drifted) => drifted,
                    Advance::Diverted(_) | Advance::Stopped(_) | Advance::Stood => pushed,
                }
            };
            if point == position {
                body.location
            } else {
                Location::new(point, body.location.size(), grid.size())
            }
        })
        .collect()
}

/// Two bodies near enough to meet this pass, `first` the lower index.
struct Pair {
    /// The lower index.
    first: usize,
    /// The higher index.
    second: usize,
    /// The offset from `first`'s middle to `second`'s.
    offset: (FixedI64, FixedI64),
    /// The distance between their middles.
    distance: FixedI64,
    /// How near their middles come when they touch.
    reach: FixedI64,
    /// The unit direction from `first` to `second`; along x when the two
    /// middles coincide.
    unit: (FixedI64, FixedI64),
    /// How they meet.
    meeting: Meeting,
}

impl Pair {
    /// The other body of the pair and how they meet, for one of its two.
    fn other(&self, body: usize) -> (usize, Meeting) {
        if body == self.first {
            (self.second, self.meeting)
        } else {
            (self.first, self.meeting)
        }
    }

    /// What the pair must part by this pass; `None` when nothing.
    fn parting(&self, bodies: &[Body]) -> Option<Parting> {
        let (first, second) = (&bodies[self.first], &bodies[self.second]);
        // Idle bodies short of their spacing by more than the slack drift,
        // never past the spacing; an overlap deeper than the slack parts at
        // once, the larger of the two standing in for both.
        let overlap = if self.reach - self.distance > TOUCH_SLACK {
            self.reach - self.distance
        } else {
            FixedI64::ZERO.min(self.reach - self.distance)
        };
        let shortfall = self.reach + contact::spacing(self.meeting, first, second) - self.distance;
        let drift = if shortfall > IDLE_SLACK {
            shortfall.min(IDLE_DRIFT)
        } else {
            FixedI64::ZERO
        };
        let total = overlap.max(drift);
        if total <= FixedI64::ZERO {
            return None;
        }
        let overlap = overlap.max(FixedI64::ZERO);
        Some(Parting {
            overlap,
            drift: total - overlap,
        })
    }
}

/// What a pair must part by this pass.
struct Parting {
    /// The overlap, parted at once.
    overlap: FixedI64,
    /// The rest of the shortfall of the idle spacing, drifted.
    drift: FixedI64,
}

/// Every pair of `bodies` sharing a layer whose circles lie within [`NEAR`]
/// of touching, in index order.
fn near_pairs(bodies: &[Body], meeting: impl Fn(usize, usize) -> Meeting) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for first in 0..bodies.len() {
        for second in (first + 1)..bodies.len() {
            if bodies[first].mover.layers & bodies[second].mover.layers == LayerMask::EMPTY {
                continue;
            }
            let from = bodies[first].location.middle();
            let to = bodies[second].location.middle();
            let distance = from.distance(to).to_num::<FixedI64>();
            let reach = (bodies[first].radius + bodies[second].radius).to_num::<FixedI64>();
            if distance >= reach + NEAR {
                continue;
            }
            let offset = (
                motion::signed(to.x) - motion::signed(from.x),
                motion::signed(to.y) - motion::signed(from.y),
            );
            pairs.push(Pair {
                first,
                second,
                offset,
                distance,
                reach,
                unit: motion::unit(offset)
                    .map_or((FixedI64::ONE, FixedI64::ZERO), |(unit, _)| unit),
                meeting: meeting(first, second),
            });
        }
    }
    pairs
}

/// How far a body can move the way a push takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Way {
    /// It stands: a wall to the bodies within [`ROOM`] of touching it.
    Shut,
    /// Part of the way.
    Part {
        /// The fraction of a push the ground lets it make along the line, from
        /// zero to one: its move over the terrain, the map's edge and its
        /// walls.
        ground: FixedI64,
        /// The fraction of a push the bodies it would press on allow along
        /// the line, from zero to one.
        chain: FixedI64,
    },
    /// The whole way.
    Open,
}

/// Which body of a pair a judgment is about.
#[derive(Debug, Clone, Copy)]
enum Side {
    /// The lower index, pushed away from the other.
    First,
    /// The higher index, pushed away from the other.
    Second,
}

impl Side {
    /// The side `body` is on in `pair`.
    fn of(pair: &Pair, body: usize) -> Side {
        if body == pair.first {
            Side::First
        } else {
            Side::Second
        }
    }

    /// Where this side's way sits in a pair's `[first, second]`.
    fn slot(self) -> usize {
        match self {
            Side::First => 0,
            Side::Second => 1,
        }
    }
}

/// Each pair's ways, `[first, second]`: each body's way away from the other,
/// judged for pairs within [`ROOM`] of touching and pairs that part this
/// pass. First the walls: every way is judged over the ground ([`footing`])
/// against the walls the last sweep's shut ways make, until a sweep shuts
/// nothing new — each way's ground part is then its move over the final
/// walls. Then the chains: each way's chain part ([`chain`]) is judged against
/// the last sweep's ways, never wider than the last ([`narrower`]), until a
/// sweep changes nothing.
fn ways(bodies: &[Body], pairs: &[Pair], near: &[Vec<usize>], grid: &NavGrid) -> Vec<[Way; 2]> {
    let judges: Vec<bool> = pairs
        .iter()
        .map(|pair| pair.distance < pair.reach + ROOM || pair.parting(bodies).is_some())
        .collect();
    let sweep = |ways: &[[Way; 2]], judge: &dyn Fn(usize, Side, Way) -> Way| -> Vec<[Way; 2]> {
        (0..pairs.len())
            .map(|index| {
                if !judges[index] {
                    return [Way::Open; 2];
                }
                [Side::First, Side::Second].map(|side| match ways[index][side.slot()] {
                    Way::Shut => Way::Shut,
                    was @ (Way::Part { .. } | Way::Open) => judge(index, side, was),
                })
            })
            .collect()
    };
    let mut ways = vec![[Way::Open; 2]; pairs.len()];
    loop {
        let judged = sweep(&ways, &|index, side, _| {
            footing(bodies, pairs, near, &ways, index, side, grid)
        });
        let shuts_alike = judged
            .iter()
            .flatten()
            .zip(ways.iter().flatten())
            .all(|(&judged, &was)| shut(judged) == shut(was));
        ways = judged;
        if shuts_alike {
            break;
        }
    }
    loop {
        let judged = sweep(&ways, &|index, side, was| {
            let ground = match was {
                Way::Part { ground, .. } => ground,
                Way::Open => FixedI64::ONE,
                Way::Shut => unreachable!("a shut way is kept"),
            };
            let chain = chain(bodies, pairs, near, &ways, index, side);
            let judged = if ground == FixedI64::ONE && chain == FixedI64::ONE {
                Way::Open
            } else {
                Way::Part { ground, chain }
            };
            narrower(was, judged)
        });
        if judged == ways {
            return ways;
        }
        ways = judged;
    }
}

/// The body on `side` of the pair at `index`, and the unit direction away
/// from its partner.
fn sliver(pairs: &[Pair], index: usize, side: Side) -> (usize, (FixedI64, FixedI64)) {
    let pair = &pairs[index];
    let (unit_x, unit_y) = pair.unit;
    match side {
        Side::First => (pair.first, (-unit_x, -unit_y)),
        Side::Second => (pair.second, (unit_x, unit_y)),
    }
}

/// The way over the ground of the body on `side` of the pair at `index`,
/// away from its partner: a sliver step of [`ROOM`] that way, moved as
/// [`motion::advance`] moves a body over the terrain and [`walls`] — shut
/// when it stands, otherwise the part of the step that move makes along the
/// way, its chain part whole; open when that is the whole step.
fn footing(
    bodies: &[Body],
    pairs: &[Pair],
    near: &[Vec<usize>],
    ways: &[[Way; 2]],
    index: usize,
    side: Side,
    grid: &NavGrid,
) -> Way {
    let (moved, (away_x, away_y)) = sliver(pairs, index, side);
    let body = &bodies[moved];
    let from = body.location.middle();
    let (from_x, from_y) = (motion::signed(from.x), motion::signed(from.y));
    let to = motion::unsigned((from_x + away_x * ROOM, from_y + away_y * ROOM));
    let beyond = walls(bodies, pairs, &near[moved], ways, moved);
    let over_ground = Ground::new(grid, body.mover, body.shape(), &beyond);
    let ground = match motion::advance(to, &over_ground) {
        Advance::Stood => return Way::Shut,
        Advance::Whole(_) => return Way::Open,
        Advance::Diverted(point) | Advance::Stopped(point) => {
            let along = (motion::signed(point.x) - from_x) * away_x
                + (motion::signed(point.y) - from_y) * away_y;
            (along / ROOM).clamp(FixedI64::ZERO, FixedI64::ONE)
        }
    };
    if ground == FixedI64::ONE {
        return Way::Open;
    }
    Way::Part {
        ground,
        chain: FixedI64::ONE,
    }
}

/// The chain part of the way of the body on `side` of the pair at `index`:
/// the part of a step away from its partner the bodies it would press on
/// allow ([`pressed_on`]), rounded up to a [`WAY_STEP`].
fn chain(
    bodies: &[Body],
    pairs: &[Pair],
    near: &[Vec<usize>],
    ways: &[[Way; 2]],
    index: usize,
    side: Side,
) -> FixedI64 {
    let (moved, away) = sliver(pairs, index, side);
    let chain = pressed_on(bodies, pairs, &near[moved], ways, index, moved, away);
    ((chain / WAY_STEP).ceil() * WAY_STEP).min(FixedI64::ONE)
}

/// How much of a step along `away` the body `moved` can make for the
/// bodies it would press on: of its partners within [`ROOM`] of touching in
/// the pairs `near` it lists, but the pair at `pushed_pair` and the walls
/// ([`walls`] already bars them), each one ahead of it at an angle whose
/// cosine to `away` is `c` allows one less `c` times what that partner
/// cannot make of its own way away from it in `ways`; the least of those,
/// the whole step when none is ahead.
fn pressed_on(
    bodies: &[Body],
    pairs: &[Pair],
    near: &[usize],
    ways: &[[Way; 2]],
    pushed_pair: usize,
    moved: usize,
    (away_x, away_y): (FixedI64, FixedI64),
) -> FixedI64 {
    near.iter()
        .filter(|&&index| index != pushed_pair)
        .filter_map(|&index| {
            let pair = &pairs[index];
            if pair.distance >= pair.reach + ROOM || pair.distance == FixedI64::ZERO {
                return None;
            }
            let (other, meeting) = pair.other(moved);
            if !contact::gives_way(bodies[other].bearing, bodies[moved].bearing, meeting) {
                return None;
            }
            let (unit_x, unit_y) = pair.unit;
            let (to_x, to_y) = match Side::of(pair, moved) {
                Side::First => (unit_x, unit_y),
                Side::Second => (-unit_x, -unit_y),
            };
            let cosine = to_x * away_x + to_y * away_y;
            if cosine <= FixedI64::ZERO {
                return None;
            }
            let partner_made = match ways[index][Side::of(pair, other).slot()] {
                Way::Shut => return None,
                way @ (Way::Part { .. } | Way::Open) => made(way),
            };
            Some(FixedI64::ONE - cosine * (FixedI64::ONE - partner_made))
        })
        .fold(FixedI64::ONE, FixedI64::min)
        .max(FixedI64::ZERO)
}

/// The narrower of two ways: each part the lesser of the two.
fn narrower(was: Way, judged: Way) -> Way {
    match (was, judged) {
        (Way::Shut, _) | (_, Way::Shut) => Way::Shut,
        (Way::Open, judged) => judged,
        (was, Way::Open) => was,
        (
            Way::Part {
                ground: was_ground,
                chain: was_chain,
            },
            Way::Part {
                ground: judged_ground,
                chain: judged_chain,
            },
        ) => Way::Part {
            ground: was_ground.min(judged_ground),
            chain: was_chain.min(judged_chain),
        },
    }
}

/// The fraction of a push a body makes along the line on `way`: the lesser
/// of its parts.
fn made(way: Way) -> FixedI64 {
    match way {
        Way::Shut => FixedI64::ZERO,
        Way::Part { ground, chain } => ground.min(chain),
        Way::Open => FixedI64::ONE,
    }
}

/// The fraction of a push a body is moved by on `way`, its move over the
/// ground making [`made`] of it: the whole push where the ground is the
/// tighter limit, the chain's part over the ground's where the chain is.
fn pushed(way: Way) -> FixedI64 {
    match way {
        Way::Shut => FixedI64::ZERO,
        Way::Part { ground, chain } => {
            if chain >= ground {
                FixedI64::ONE
            } else {
                chain / ground
            }
        }
        Way::Open => FixedI64::ONE,
    }
}

/// The circles `body` may not enter, of the partners in the pairs `near` it
/// lists: those that do not give way to it, and those within [`ROOM`] of
/// touching shut toward it in `ways`.
fn walls(
    bodies: &[Body],
    pairs: &[Pair],
    near: &[usize],
    ways: &[[Way; 2]],
    body: usize,
) -> Vec<Shape> {
    near.iter()
        .filter_map(|&index| {
            let pair = &pairs[index];
            let (other, meeting) = pair.other(body);
            let is_wall = !contact::gives_way(bodies[other].bearing, bodies[body].bearing, meeting)
                || pair.distance < pair.reach + ROOM
                    && shut(ways[index][Side::of(pair, other).slot()]);
            is_wall.then(|| bodies[other].shape())
        })
        .collect()
}

/// Whether `way` is shut.
fn shut(way: Way) -> bool {
    match way {
        Way::Shut => true,
        Way::Part { .. } | Way::Open => false,
    }
}

/// What a body takes of its partner's share.
#[derive(Debug, Clone, Copy)]
enum Handover {
    /// This much its partner cannot make along the line.
    Takes(FixedI64),
    /// Nothing: it moves its own share only.
    Keeps,
}

/// A body's displacement from its pairs, before it is moved, each part held
/// to [`MAX_PUSH`] per axis.
struct Push {
    /// What its overlaps push it by, swerves, nudges and leans included.
    overlap: FixedVec2,
    /// What it drifts by toward the idle spacing; none for a squeezed body.
    drift: FixedVec2,
}

/// Each body's displacement from its pairs, per [`settle`], before it is
/// moved.
fn pushes(bodies: &[Body], pairs: &[Pair], ways: &[[Way; 2]]) -> Vec<Push> {
    let mut pushes = vec![FixedVec2::ZERO; bodies.len()];
    // The drift part of each body's push, and the directions it is pushed
    // from, kept apart until every pair is read: a body pushed from opposing
    // sides takes none of the drift.
    let mut drifts = vec![FixedVec2::ZERO; bodies.len()];
    let mut pushed_from: Vec<Vec<(FixedI64, FixedI64)>> = vec![Vec::new(); bodies.len()];
    for (pair, &[first_way, second_way]) in pairs.iter().zip(ways) {
        let Some(Parting { overlap, drift }) = pair.parting(bodies) else {
            continue;
        };
        let (first, second) = (&bodies[pair.first], &bodies[pair.second]);
        let (dx, dy) = pair.offset;
        let (base_first, base_second) =
            contact::shares(pair.meeting, overlap, first, second, dx, dy);
        // What each body makes of its share along the line, and what it
        // cannot.
        let left = |base: FixedI64, way: Way| base - base * made(way);
        let first_left = left(base_first, first_way);
        let second_left = left(base_second, second_way);
        let handover = |taker: &Body, way: Way, other: &Body, left: FixedI64| {
            if left > FixedI64::ZERO
                && !shut(way)
                && contact::gives_way(taker.bearing, other.bearing, pair.meeting)
            {
                Handover::Takes(left)
            } else {
                Handover::Keeps
            }
        };
        let swerve = contact::swerves(first, second, dx, dy);
        // Each body's radial and sideways shares: its own share and what it
        // takes of its partner's, moved by its way — making `made` of them
        // along the line, what it cannot make of the taken share staying —
        // swerving a quarter of what it makes of its own past a walker it
        // must pass, nudged aside off a walker's line on its whole share, and
        // leaning past the partner on what it makes of what it takes.
        let share = |body: &Body,
                     other: &Body,
                     base: FixedI64,
                     way: Way,
                     handover: Handover,
                     dx: FixedI64,
                     dy: FixedI64| {
            let swerving = if swerve {
                base * made(way) / FixedI64::from_num(4)
            } else {
                FixedI64::ZERO
            };
            let nudged = contact::aside(body, other, base, dx, dy, pair.distance);
            match handover {
                Handover::Takes(left) => (
                    (base + left) * pushed(way),
                    swerving
                        + nudged
                        + contact::lean(body, left * made(way), dx, dy, pair.distance),
                ),
                Handover::Keeps => (base * pushed(way), swerving + nudged),
            }
        };
        let first_handover = handover(first, first_way, second, second_left);
        let second_handover = handover(second, second_way, first, first_left);
        let (first_radial, first_lateral) =
            share(first, second, base_first, first_way, first_handover, dx, dy);
        let (second_radial, second_lateral) = share(
            second,
            first,
            base_second,
            second_way,
            second_handover,
            -dx,
            -dy,
        );
        let (unit_x, unit_y) = pair.unit;
        // Each side's sideways share is the clockwise perpendicular of its own
        // outward direction when positive.
        pushes[pair.first].x -= unit_x * first_radial + unit_y * first_lateral;
        pushes[pair.first].y -= unit_y * first_radial - unit_x * first_lateral;
        pushes[pair.second].x += unit_x * second_radial + unit_y * second_lateral;
        pushes[pair.second].y += unit_y * second_radial - unit_x * second_lateral;
        // A body pressed from a side is pressed from it whether it can give
        // way or not.
        if base_first > FixedI64::ZERO {
            pushed_from[pair.first].push((-unit_x, -unit_y));
        }
        if base_second > FixedI64::ZERO {
            pushed_from[pair.second].push((unit_x, unit_y));
        }
        if drift > FixedI64::ZERO {
            let (first_drift, second_drift) =
                contact::shares(pair.meeting, drift, first, second, dx, dy);
            // A body drifted from a side is pressed from it whether it can
            // give way or not, as with an overlap.
            if first_drift > FixedI64::ZERO {
                pushed_from[pair.first].push((-unit_x, -unit_y));
            }
            if second_drift > FixedI64::ZERO {
                pushed_from[pair.second].push((unit_x, unit_y));
            }
            let first_drift = first_drift * pushed(first_way);
            let second_drift = second_drift * pushed(second_way);
            drifts[pair.first].x -= unit_x * first_drift;
            drifts[pair.first].y -= unit_y * first_drift;
            drifts[pair.second].x += unit_x * second_drift;
            drifts[pair.second].y += unit_y * second_drift;
        }
    }
    let clamped = |push: FixedVec2| {
        FixedVec2::new(
            push.x.clamp(-MAX_PUSH, MAX_PUSH),
            push.y.clamp(-MAX_PUSH, MAX_PUSH),
        )
    };
    pushes
        .into_iter()
        .zip(drifts)
        .zip(&pushed_from)
        .map(|((overlap, drift), from)| Push {
            overlap: clamped(overlap),
            drift: if squeezed(from) {
                FixedVec2::ZERO
            } else {
                clamped(drift)
            },
        })
        .collect()
}

/// Whether pushes along the unit directions `from` squeeze a body: two of
/// them more than 120° apart.
fn squeezed(from: &[(FixedI64, FixedI64)]) -> bool {
    from.iter().enumerate().any(|(index, &(x, y))| {
        from[index + 1..]
            .iter()
            .any(|&(other_x, other_y)| x * other_x + y * other_y < -SQUEEZE_OPPOSITION)
    })
}

/// `position` moved by `push`, held at the map's origin.
fn displaced(position: FixedUVec2, push: FixedVec2) -> FixedUVec2 {
    motion::unsigned((
        motion::signed(position.x) + push.x,
        motion::signed(position.y) + push.y,
    ))
}
