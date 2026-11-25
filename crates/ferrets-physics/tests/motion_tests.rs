//! Moving one body over its ground: sliding around what it meets, stopping
//! where it first touches, and standing when nothing is admitted.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, projection::Projection, shape::Shape};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_nav_grid::nav_grid::NavGrid;
use ferrets_physics::{
    ground::Ground,
    motion::{self, Advance},
};

//
// ─── Stepping around ──────────────────────────────────────────────────────────
//

/// A step that leaves the walker clear of the obstacle is kept whole.
#[test]
fn step_clear_of_obstacle_is_kept() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacle = utils::resting("5.5", "5.5").shape();

    let reached = step(walker, utils::point("3", "5.5"), obstacle);

    // 5.5 − 3 = 2.5 apart, past the touching 0.5 + 0.5 = 1.
    assert_eq!(reached, utils::point("3", "5.5"));
}

/// A step into the obstacle stops where the walk first touches it and slides
/// the rest along the tangent there: from (4, 5.5) toward (4.5, 5.5), the
/// obstacle at (5, 5), the walk touches at t = (0.5 − √0.1875) / 0.25 ≈
/// 0.268, at (4.134, 5.5); the rest, 0.732 of the step, carries
/// 0.732 × 0.5 × 0.5 = 0.183 along the tangent (0.5, 0.866), to
/// (4.2255, 5.6585), 1.017 from the middle.
#[test]
fn step_into_obstacle_slides_along_rim_from_first_touch() {
    let walker = utils::resting("4", "5.5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let reached = step(walker, utils::point("4.5", "5.5"), obstacle);

    assert_eq!(reached, utils::point("4.225480947", "5.6584936487"));
}

/// A step aimed dead at the obstacle's middle touches at its midpoint,
/// (4, 5), then slides a quarter of what is left clockwise:
/// 0.5 × 0.5 / 4 = 0.0625, to (4, 5.0625).
#[test]
fn dead_on_step_turns_clockwise() {
    let walker = utils::resting("3.75", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let reached = step(walker, utils::point("4.25", "5"), obstacle);

    assert_eq!(reached, utils::point("4", "5.0625"));
}

/// A step aimed all but dead at the obstacle's middle — 0.0001 off — slides
/// at least a quarter of what is left the way its aim leans, rather than
/// creeping along the rim by the sliver it leans.
#[test]
fn near_dead_on_step_slides_at_least_quarter() {
    let walker = utils::resting("3.75", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let reached = step(walker, utils::point("4.25", "5.0001"), obstacle);

    // Touching near (4, 5), it slides 0.5 × 0.5 / 4 = 0.0625 clockwise, to
    // about (4, 5.0625); creeping by its lean alone it would reach only
    // (4, 5.0001).
    assert_eq!(reached, utils::point("4.000003126", "5.0625500001"));
}

/// The quarter follows the lean: aimed 0.0001 below the middle, the step
/// slides the quarter the other way, counterclockwise.
#[test]
fn near_dead_on_step_slides_way_it_leans() {
    let walker = utils::resting("3.75", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    let reached = step(walker, utils::point("4.25", "4.9999"), obstacle);

    // Touching near (4, 5), it slides 0.5 × 0.5 / 4 = 0.0625
    // counterclockwise, to about (4, 4.9375).
    assert_eq!(reached, utils::point("4.000003126", "4.9374499992"));
}

/// A walker rounds a body in its way at a quarter of its step at least:
/// 0.1 steps aimed dead through an obstacle to a point behind it come level
/// with the obstacle's middle in a pinned number of steps, where sliding
/// only by the part of each step along the tangent creeps around for many
/// more.
#[test]
fn walker_rounds_obstacle_at_quarter_step_at_least() {
    let obstacle = utils::resting("5", "5").shape();
    let aim = utils::point("7", "5");
    let mut position = utils::point("3.9", "5");
    let mut steps = 0;
    while position.x < utils::cells("5") {
        let walker = Shape::Circle {
            middle: position,
            radius: utils::cells("0.5"),
        };
        position = step(
            walker,
            Projection::Orthogonal.step_toward(position, aim, utils::cells("0.1")),
            obstacle,
        );
        steps += 1;
        assert!(steps < 200, "the walker never got around");
    }

    // Forty steps: one to the touch, then around the rim at a quarter of each
    // step at least while the aim leans past the obstacle, more as it leans
    // further; sliding only by the part of each step along the tangent it
    // takes sixty-eight.
    assert_eq!(steps, 40);
}

/// Two equal bodies on one point are one rim to slide along: a walker meeting
/// them slides past exactly as past one.
#[test]
fn walker_slides_past_stacked_bodies_as_past_one() {
    let walker = utils::resting("2.5", "5.5").shape();
    let body = utils::resting("4.5", "5.75").shape();
    let grid = utils::open_grid();
    let past = |obstacles: &[Shape]| {
        motion::advance(
            utils::point("8.5", "5.5"),
            &Ground::new(&grid, utils::ground_shape(), walker, obstacles),
        )
    };

    // Its principle, the result with one body: touching at x 4.5 −
    // √(1 − 0.25²) ≈ 3.532, the way out (−0.968, −0.25), the tangent
    // (−0.25, 0.968); the rest of the walk, 6 × 0.828 ≈ 4.97, carries −1.242
    // along it — its quarter — so the slide is (3.532 + 0.3105, 5.5 − 1.2025).
    assert_eq!(past(&[body, body]), past(&[body]));
    assert_eq!(
        past(&[body]),
        Advance::Diverted(utils::point("3.8422695275", "4.2973791633"))
    );
}

/// A walker all but on a body's middle slides along a rim as tight as its
/// depth as off any rim: a speck 3 bits east of a body's middle, stepping 0.1
/// west and 0.02 north across it, slides a quarter of its walk north around
/// that rim.
#[test]
fn walker_near_middle_slides_as_off_any_rim() {
    let walker = Shape::Circle {
        middle: utils::point_bits(0x5_0000_0003, 0x5_0000_0000),
        radius: utils::cells("0.1"),
    };
    let obstacles = [utils::resting("5", "5").shape()];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point_bits(0x4_e666_6669, 0x5_051e_b852), &ground);

    // Met at once, the way out east, the tangent south: the step carries 0.02
    // against it, less than a quarter of the walk, √(0.1² + 0.02²) / 4 ≈
    // 0.0255, so the slide is that quarter, north.
    assert_eq!(
        reached,
        Advance::Diverted(utils::point("5.0000000007", "5.0254950975"))
    );
}

/// A slide's tangent is a unit however few bits the way out spans: a speck 3
/// bits east and 1 bit north of a body's middle, stepping 0.03 east and 0.1
/// south across it, slides no farther than its step.
#[test]
fn slide_off_few_bit_way_out_stays_within_step() {
    let walker = Shape::Circle {
        middle: utils::point_bits(0x5_0000_0003, 0x5_0000_0001),
        radius: utils::cells("0.1"),
    };
    let obstacles = [utils::resting("5", "5").shape()];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point_bits(0x5_07ae_147d, 0x4_e666_6668), &ground);

    // Met at once, the way out (3, 1)/√10 ≈ (0.9487, 0.3162), the tangent
    // clockwise of it (0.3162, −0.9487): the step carries 0.03 × 0.3162 +
    // 0.1 × 0.9487 ≈ 0.10436 along it, within the walk's √(0.03² + 0.1²) ≈
    // 0.10440, so the slide lands ≈ (5 + 0.0330, 5 − 0.0990). A tangent read
    // from (3, 1) over a floored length of 3 would be 1.054 long, the slide
    // ≈ 0.116.
    assert_eq!(
        reached,
        Advance::Diverted(utils::point("5.0330000005", "4.9010000005"))
    );
}

/// A body boxed in stands: a body under a wall, touching another flush on
/// the wall closing the pocket east, tried a sliver north-east, stands.
#[test]
fn boxed_body_stands() {
    let walker = utils::resting("6.5", "5.5").shape();
    let obstacles = [utils::resting("7.5", "5.5").shape()];
    let mut grid = utils::open_grid();
    for x in 0..13 {
        grid.set_occupied(utils::GROUND, CellPos::new(x, 6), true);
    }
    grid.set_occupied(utils::GROUND, CellPos::new(8, 5), true);
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    // A 1/64 sliver at ≈ 72°: (0.0046875, 0.0149053), √(0.0046875² +
    // 0.0149053²) ≈ 0.015625. North is the wall row, east the body flush on
    // the wall; the slide along it and every axis drop are refused.
    let reached = motion::advance(utils::point("6.5046875", "5.5149053"), &ground);

    assert_eq!(reached, Advance::Stood);
}

/// A crumb short of a wall is no move: a body 2⁻²⁵ west of flush against a
/// wall east, probed a sliver east into it, is admitted only the 2⁻²⁵ up to
/// flush, under the 2⁻²⁴ a move must cover, so it stands.
#[test]
fn crumb_short_of_wall_is_no_move() {
    let walker = Shape::Circle {
        middle: utils::point_bits(0x6_7fff_ff80, 0x5_8000_0000),
        radius: utils::cells("0.5"),
    };
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(7, 5), true);
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &[]);

    // The probe, 1/64 east: 2²⁶ bits, halved to 2⁻²⁶ steps; flush at 6.5 is
    // two of them, 2⁻²⁵ = 0x80 bits.
    let reached = motion::advance(utils::point_bits(0x6_83ff_ff80, 0x5_8000_0000), &ground);

    assert_eq!(reached, Advance::Stood);
}

/// A walk only bits long still takes its bits: a body 2⁻²⁶ west of flush
/// against a wall east, walking 2⁻²⁵ east into it, stops flush, the whole
/// walk shorter than the 2⁻²⁴ a longer walk must cover.
#[test]
fn bits_long_walk_takes_its_bits() {
    let walker = Shape::Circle {
        middle: utils::point_bits(0x6_7fff_ffc0, 0x5_8000_0000),
        radius: utils::cells("0.5"),
    };
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(7, 5), true);
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &[]);

    // 0x80 bits east, flush at 0x40 of them: the first halving.
    let reached = motion::advance(utils::point_bits(0x6_8000_0040, 0x5_8000_0000), &ground);

    assert_eq!(reached, Advance::Stopped(utils::point("6.5", "5.5")));
}

/// A walk grazing a rim within 2⁻²⁴ does not meet it: a body walking east
/// two cells, passing a body of the same radius a bit under 1 off its line,
/// comes no deeper than touching and takes its whole walk.
#[test]
fn walk_grazing_rim_within_slack_is_whole() {
    let walker = Shape::Circle {
        middle: utils::point_bits(0x4_0000_0000, 0x4_8000_0001),
        radius: utils::cells("0.5"),
    };
    let obstacles = [utils::resting("5", "5.5").shape()];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    // 1 − 2⁻³² off the walk's line, 2⁻³² inside the 0.5 + 0.5 = 1 of
    // touching.
    let aim = utils::point_bits(0x6_0000_0000, 0x4_8000_0001);
    let reached = motion::advance(aim, &ground);

    assert_eq!(reached, Advance::Whole(aim));
}

//
// ─── Stepping around small bodies ─────────────────────────────────────────────
//

/// A step long against small bodies stops at the first touch: radius-0.1
/// bodies touch 0.2 apart; from 0.3 off, a 0.3 step aimed dead at the
/// obstacle's middle touches a third of the way, at 0.2 off, and slides a
/// quarter of the rest, (2/3 × 0.3) / 4 = 0.05, clockwise — on the near side.
#[test]
fn long_step_stops_at_first_touch_on_near_side() {
    let walker = speck("4.7", "5");
    let obstacle = speck("5", "5");

    let reached = step(walker, utils::point("5", "5"), obstacle);

    // (4.8, 5.05) but for the last bits: 0.3 is not exact in binary.
    assert_eq!(reached, utils::point_bits(0x4_cccc_cccc, 0x5_0ccc_cccc));
}

/// A step that would carry the walker clean over the obstacle stops at the
/// first touch too: a 0.7 step from 0.3 off touches at t = 1/7, at 0.2 off,
/// and slides a quarter of the rest, (6/7 × 0.7) / 4 = 0.15, never past the
/// step: 0.2 + 0.15 ≤ 0.7.
#[test]
fn step_never_jumps_obstacle() {
    let walker = speck("4.7", "5");
    let obstacle = speck("5", "5");

    let reached = step(walker, utils::point("5.4", "5"), obstacle);

    // (4.8, 5.15) but for the last bits: 0.3 and 0.7 are not exact in
    // binary.
    assert_eq!(reached, utils::point_bits(0x4_cccc_cccc, 0x5_2666_6666));
}

/// A walker already overlapping the obstacle takes a step that leaves it no
/// deeper, and meets it at once on one that would sink it further, sliding
/// along its rim where it stands: 0.5 from the middle, a step to 0.75 is
/// taken, and one 0.25 toward the middle slides a quarter of itself, 0.0625,
/// clockwise of the way out — never deeper.
#[test]
fn overlapping_walker_never_steps_deeper() {
    let walker = utils::resting("4.5", "5").shape();
    let obstacle = utils::resting("5", "5").shape();

    assert_eq!(
        step(walker, utils::point("4.25", "5"), obstacle),
        utils::point("4.25", "5")
    );
    assert_eq!(
        step(walker, utils::point("4.75", "5"), obstacle),
        utils::point("4.5", "5.0625")
    );
}

/// A grazing diagonal walk that would sink into an obstacle stops where it
/// first touches it, backed off along the walk's own line: from (2, 2)
/// toward (2.8, 2.1), past an obstacle whose rim the walk grazes, the stop
/// lies on the walk.
#[test]
fn grazing_first_touch_stays_on_walk_line() {
    let walker = utils::resting("2", "2").shape();
    let obstacles = [utils::resting("3", "2.9").shape()];
    let mut grid = utils::open_grid();
    // Walls close every slide: only the walk's own line is left.
    for (x, y) in [(2, 1), (3, 1), (2, 3), (1, 2)] {
        grid.set_occupied(utils::GROUND, CellPos::new(x, y), true);
    }
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("2.8", "2.1"), &ground);

    // On the line y = 2 + 0.125 (x − 2): at x 2.4611, y 2.0576.
    assert_eq!(
        reached,
        Advance::Stopped(utils::point("2.4610905237", "2.0576363155"))
    );
}

//
// ─── Terrain and edges ────────────────────────────────────────────────────────
//

/// Where the slide around a body is refused, the walker stops where it first
/// touches: in a corridor one cell high, a walker meeting an obstacle a
/// tenth of a cell off its line would slide sideways into the walls, and
/// with the sideways part dropped would sink into the obstacle; it stops at
/// the first touch instead, x = 3.05 − √(1 − 0.1²) ≈ 2.055, touching it.
#[test]
fn refused_rim_slide_stops_at_first_touch() {
    let walker = utils::resting("2", "5.5").shape();
    let obstacles = [utils::resting("3.05", "5.6").shape()];
    let grid = utils::corridor_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("2.1", "5.5"), &ground);

    assert_eq!(
        reached,
        Advance::Stopped(utils::point("2.0550125628", "5.5"))
    );
}

/// A fallback never jumps a body the walk meets: in a corridor one cell
/// high, a walker touching an obstacle dead ahead and aimed 3 cells past it
/// slides into the walls, stops at a touch it already stands on, and the aim
/// with the cross axis dropped — the aim itself — lies past the obstacle, its
/// walk through it; so it stands.
#[test]
fn walk_through_touched_obstacle_stands() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacles = [utils::resting("3.5", "5.5").shape()];
    let grid = utils::corridor_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("5.5", "5.5"), &ground);

    assert_eq!(reached, Advance::Stood);
}

/// A slide around one body never jumps another: a walker meeting a body 2
/// cells ahead on a 6-cell step slides up its rim, but the walk to that
/// slide, and to its point with x dropped, crosses a speck; the slide with y
/// dropped is clear, and taken.
#[test]
fn slide_never_jumps_other_body() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacles = [utils::resting("4.5", "5.5").shape(), speck("3", "6.125")];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("8.5", "5.5"), &ground);

    // Touching the body at 4.5 − 1 = 3.5; the slide up its rim and that
    // point with x dropped both walk through the speck; with y dropped, the
    // walk along the line to (3.5, 5.5) is clear — but for the rim's
    // rounding.
    assert_eq!(
        reached,
        Advance::Diverted(utils::point("3.499999999", "5.5"))
    );
}

/// An axis-dropped slide is walked clear of the body slid along too: a
/// walker grazing a body, its slide refused by a wall row above, would reach
/// the slide with y dropped only through that body's rim, so it stops where
/// it touches it.
#[test]
fn dropped_slide_never_cuts_rim_it_slides_along() {
    let quarter = utils::cells("0.25");
    let walker = Shape::Circle {
        middle: utils::point("4.74", "5.6"),
        radius: quarter,
    };
    let obstacles = [Shape::Circle {
        middle: utils::point("5", "5.167"),
        radius: quarter,
    }];
    let mut grid = utils::open_grid();
    for x in 0..12 {
        grid.set_occupied(utils::GROUND, CellPos::new(x, 6), true);
    }
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("5.44", "5.6"), &ground);

    // 0.433 off the walk's line, the middles touch 0.25 + 0.25 = 0.5 apart:
    // at x 5 − √(0.5² − 0.433²) ≈ 5 − 0.250022 ≈ 4.749978 (0.433 is a hair
    // under √3 / 4), on the rim, so not backed off. The way out is (−0.5,
    // 0.866), the tangent clockwise of it (0.866, 0.5); the rest of the walk,
    // 0.69, carries 0.69 × 0.866 ≈ 0.5975 along it, so the slide reaches
    // (4.75 + 0.5175, 5.6 + 0.299), in the wall row; with y dropped it lies
    // at (5.2675, 5.6), 0.509 from the middle, but the walk there passes
    // 0.433 from it, inside the rim.
    assert_eq!(
        reached,
        Advance::Stopped(utils::point("4.7499780012", "5.6"))
    );
}

/// A touch point the ground refuses is tried with an axis dropped, each walked
/// clear of the other bodies: a walker heading north-east at a body, its
/// touch point and the slides past it in walls, would reach the touch with y
/// dropped only through a speck, and the touch with x dropped lies in a wall,
/// so it stops as far along its walk as the walls let it.
#[test]
fn dropped_touch_never_jumps_other_body() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacles = [utils::resting("4.5", "7.5").shape(), speck("3.15", "5.2")];
    let mut grid = utils::open_grid();
    for (x, y) in [(3, 7), (4, 7), (2, 7), (2, 9)] {
        grid.set_occupied(utils::GROUND, CellPos::new(x, y), true);
    }
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("6.5", "9.5"), &ground);

    // The touch at (3.7929, 6.7929), 2√2 − 1 along the diagonal; with y
    // dropped, (3.7929, 5.5), the walk east passes the speck 0.3 off its line,
    // inside 0.5 + 0.1. The walls in row 7 hold the walk below y 6.5.
    assert_eq!(
        reached,
        Advance::Stopped(utils::point("3.4999992214", "6.4999992214"))
    );
}

/// The farthest point along a walk never lies past a body it meets: in a
/// corridor one cell high, a walker touching a speck dead ahead, aimed 3
/// cells past it, has every slide refused by the walls and its first touch
/// where it stands; halving the walk would land past the speck, so it
/// stands.
#[test]
fn farthest_point_never_lies_past_touched_speck() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacles = [speck("3.1", "5.5")];
    let grid = utils::corridor_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("5.5", "5.5"), &ground);

    assert_eq!(reached, Advance::Stood);
}

/// A step of any length meets a body of any size where it touches it: a
/// walker stepping 60 cells dead at a radius-1,000 body 9.5 beyond touching
/// it meets it after 9.5 of them and slides along its rim.
#[test]
fn long_step_meets_large_body_where_it_touches() {
    let walker = Shape::Circle {
        middle: utils::point("2000.5", "3000.5"),
        radius: utils::cells("0.5"),
    };
    let obstacle = Shape::Circle {
        middle: utils::point("3010.5", "3000.5"),
        radius: utils::cells("1000"),
    };
    let mut grid = NavGrid::new(4096, 4096, |_| utils::LAND);
    grid.add_flat_layer(utils::GROUND, 1);

    let reached = motion::advance(
        utils::point("2060.5", "3000.5"),
        &Ground::new(&grid, utils::ground_shape(), walker, &[obstacle]),
    );

    // Touching at x 3010.5 − 1000.5 = 2010, 9.5 into the step; the rest,
    // 50.5, slides a quarter of itself, 12.625, clockwise along the rim — but
    // for the rounding of a step this long.
    assert_eq!(
        reached,
        Advance::Diverted(utils::point("2009.9999999925", "3013.1250000019"))
    );
}

/// A step of a single bit toward a body half a cell ahead is taken whole:
/// the walk falls short of the touch, so no fraction of it is measured.
#[test]
fn bit_long_step_short_of_body_is_whole() {
    let walker = utils::resting("2.5", "5.5").shape();
    let obstacle = utils::resting("4", "5.5").shape();
    // 0x2_8000_0001 = 2.5 + 2⁻³²: one bit east. The rims touch with the
    // middles 0.5 + 0.5 = 1 apart, at x 3, 0.5 ahead — far beyond the step.
    let aim = utils::point_bits(0x2_8000_0001, 0x5_8000_0000);

    let obstacles = [obstacle];
    let grid = utils::open_grid();

    let reached = motion::advance(
        aim,
        &Ground::new(&grid, utils::ground_shape(), walker, &obstacles),
    );

    assert_eq!(reached, Advance::Whole(aim));
}

/// A walk never passes through a body it already overlaps: a speck walker
/// overlapping a speck, stepping half a cell through its middle, meets it at
/// once and slides along it where it stands rather than landing past it.
#[test]
fn walk_never_passes_through_overlapped_body() {
    let walker = speck("3", "5");
    let obstacles = [speck("3.1", "5")];
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &obstacles);

    let reached = motion::advance(utils::point("3.5", "5"), &ground);

    // Met at once, 0.1 from its middle; the step, aimed through it, slides a
    // quarter of itself, 0.5 / 4 = 0.125, clockwise of the way out (−x):
    // toward +y.
    assert_eq!(reached, Advance::Diverted(utils::point("3", "5.125")));
}

/// A walk past the map's edge stops on it, as at any obstacle: from x 1.5
/// toward x 0.2, a one-cell body stops with its middle half a cell in.
#[test]
fn walk_past_origin_stops_at_its_edge() {
    let walker = utils::resting("1.5", "3.5").shape();
    let grid = utils::open_grid();
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &[]);

    let reached = motion::advance(utils::point("0.2", "3.5"), &ground);

    // 0.5 but for the halving: twenty halvings of the 1.3-cell walk land
    // within 1.3 / 2²⁰ ≈ 0.0000012 of it, on the admitted side.
    assert_eq!(
        reached,
        Advance::Stopped(utils::point("0.5000011444", "3.5"))
    );
}

/// A walk into a wall stops against it rather than standing short: from x
/// 4.2 toward x 4.8 beside a wall at column 5, the one-cell body stops with
/// its edge on the wall, its middle at 4.5.
#[test]
fn walk_into_wall_stops_against_it() {
    let walker = utils::resting("4.2", "5.5").shape();
    let mut grid = utils::open_grid();
    grid.set_occupied(utils::GROUND, CellPos::new(5, 5), true);
    let ground = Ground::new(&grid, utils::ground_shape(), walker, &[]);

    let reached = motion::advance(utils::point("4.8", "5.5"), &ground);

    // The 0.6-cell walk halves onto 4.5 exactly: 0.3 / 0.6 = 1/2.
    assert_eq!(reached, Advance::Stopped(utils::point("4.5", "5.5")));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Where a body whose circle is `walker` gets toward `aim` past `obstacle`,
/// over open ground.
fn step(walker: Shape, aim: FixedUVec2, obstacle: Shape) -> FixedUVec2 {
    let obstacles = [obstacle];
    let grid = utils::open_grid();
    motion::advance(
        aim,
        &Ground::new(&grid, utils::ground_shape(), walker, &obstacles),
    )
    .point(walker.middle())
}

/// A radius-0.1 circle with its middle at `(x, y)`.
fn speck(x: &str, y: &str) -> Shape {
    Shape::Circle {
        middle: utils::point(x, y),
        radius: utils::cells("0.1"),
    }
}
