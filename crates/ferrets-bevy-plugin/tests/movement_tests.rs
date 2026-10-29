//! Movement that the plan alone does not settle: a continuous body pressed into
//! a building corner must round it rather than converge on the corner's tangent
//! line forever and must look where it steps while doing it, and a cell walk
//! planned as a corridor must follow the corridor rather than stop where its
//! first leg ran out.

mod utils;

use bevy::prelude::*;
use ferrets_content::registry::ContentRegistry;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_physics::terrain;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        location::LocationComponent, movement::MoveComponent, order_queue::OrderQueueComponent,
    },
    entity_def,
    map::Map,
    movement_model::MovementModel,
    order::Order,
    session::{player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

//
// ─── Corner rounding ────────────────────────────────────────────────────────
//

#[test]
fn body_pressed_into_footprint_corner_rounds_it() {
    let mut app = utils::corner_app();
    let runner = runner_clipping_keep_corner(&mut app);
    walk_to_spot(&mut app, runner, utils::pos(4, 5));

    utils::run_ticks(&mut app, 300);

    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must finish instead of grinding at the corner"
    );
    assert_eq!(
        utils::position_of(app.world_mut(), runner),
        utils::pos(4, 5),
        "the runner rounded the corner and reached the ordered spot"
    );
}

#[test]
fn body_pressed_into_water_corner_rounds_it_on_land() {
    // The keep's corner as water, (8..=10, 6..=8), and a landlubber, its
    // domain taking land alone, walking past it from where the runner of the
    // keep test starts.
    let water: Vec<(u32, u32)> = (6..=8)
        .flat_map(|y| (8..=10).map(move |x| (x, y)))
        .collect();
    let mut app = utils::dry_orders_app(MovementModel::Continuous, &water, |_, _| {});
    let (landlubber, _) = utils::create_owned(&mut app, "landlubber", 12, 5, 0);
    app.world_mut()
        .entity_mut(landlubber)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = utils::part_way("10.68", "5.08");
    walk_to_spot(&mut app, landlubber, utils::pos(4, 5));
    let shape = {
        let world = app.world();
        let location = entity_def::of(world, landlubber)
            .location
            .expect("a landlubber has a location");
        entity_def::mover_shape(world.resource::<ContentRegistry>(), &location)
    };
    let radius = entity_def::radius(app.world(), landlubber);

    // It starts clipping the water, as the keep test's runner clips the keep,
    // and still clips it after its first tick; its second squares it onto
    // row 5, and from then on its circle lies on land every tick.
    utils::run_ticks(&mut app, 1);
    for tick in 1..300 {
        utils::run_ticks(&mut app, 1);
        let position = utils::position_of(app.world_mut(), landlubber);
        assert!(
            terrain::body_fits(
                app.world().resource::<Map>().nav_grid(),
                shape,
                position,
                radius
            ),
            "on tick {tick} the landlubber's circle at {position:?} reaches over water"
        );
    }

    assert!(utils::order_queue_is_empty(app.world_mut(), landlubber));
    assert_eq!(
        utils::position_of(app.world_mut(), landlubber),
        utils::pos(4, 5)
    );
}

/// Rounding that corner, the body looks where it goes: a look taken from the aim
/// instead points into the obstacle the step just failed to enter. A crumb is no
/// heading, though — a body squaring onto a lattice point slides a pixel or two
/// along an axis it does not mean to travel, and looking down it would spin the
/// body for a tick over a movement nobody can see.
#[test]
fn body_rounding_footprint_corner_looks_where_it_steps() {
    let mut app = utils::corner_app();
    let runner = runner_clipping_keep_corner(&mut app);
    walk_to_spot(&mut app, runner, utils::pos(4, 5));

    // A quarter of a tick's walk is the floor a step has to clear to be a
    // heading, so the runner's own speed sets it.
    let crumb = utils::effective_speed(&app, runner) * FixedU64::from_num(0.25);
    let mut before = utils::position_of(app.world_mut(), runner);
    let mut looked = utils::facing_of(app.world(), runner);
    let mut crumbs = 0;
    for tick in 0..300 {
        if utils::order_queue_is_empty(app.world_mut(), runner) {
            break;
        }
        utils::run_ticks(&mut app, 1);
        let after = utils::position_of(app.world_mut(), runner);
        let step = utils::offset(before, after);
        let travelled = before.distance(after);
        before = after;
        let facing = utils::facing_of(app.world(), runner);
        if travelled < crumb {
            crumbs += 1;
            assert_eq!(
                facing, looked,
                "tick {tick}: a crumb step of {step:?} turned the body"
            );
            continue;
        }
        // The whole rule, exactly: the look a step is worth seeing for *is* that
        // step. The runner turns a whole circle a tick, so the look it wants is
        // the look it reaches — a slower body would be part way round instead.
        assert_eq!(
            facing,
            Facing::of(step).expect("a step worth seeing has a bearing"),
            "tick {tick}: stepped one way, looked another"
        );
        looked = facing;
    }
    assert_eq!(
        crumbs, 1,
        "the corner is rounded with one crumb step, which is what pins the crumb rule"
    );
}

/// A wall across the way is walked along, not grazed. A step aimed into it
/// leaves the open axis only the share that aim gave it — a fraction of the
/// walk — so the body creeps toward the corner's tangent, and creeps in a
/// direction that flips every tick the blocked axis frees. The whole step
/// belongs on the way that is open, which is also what clears the block.
#[test]
fn body_walled_across_its_way_walks_open_axis() {
    let mut app = utils::corner_app();
    let runner = runner_clipping_keep_corner(&mut app);
    let spot = utils::pos(4, 5);
    walk_to_spot(&mut app, runner, spot);
    let speed = utils::effective_speed(&app, runner);

    // The tick the corner blocks the walk spends itself on the open axis,
    // landing on the row the waypoint sits on rather than a crumb of the way
    // toward it.
    let mut before = utils::position_of(app.world_mut(), runner);
    let walled = loop {
        utils::run_ticks(&mut app, 1);
        let after = utils::position_of(app.world_mut(), runner);
        assert_ne!(after, before, "the walk must not stall on the corner");
        let stepped_west = after.x != before.x;
        before = after;
        if !stepped_west {
            break after;
        }
    };
    assert_eq!(
        walled,
        FixedUVec2::new(walled.x, FixedU64::from_num(5)),
        "the blocked tick closed the whole gap to the waypoint's own row"
    );

    // And from there the way is clear, so every step is the whole step the walk
    // asked for — one direction at its own speed, rather than the two it
    // alternated between while grazing.
    let projection = app.world().resource::<Map>().projection();
    for tick in 0..5 {
        utils::run_ticks(&mut app, 1);
        let after = utils::position_of(app.world_mut(), runner);
        assert_eq!(
            after,
            projection.step_toward(before, spot, speed),
            "tick {tick} after rounding the corner"
        );
        before = after;
    }
}

//
// ─── Walled point orders ──────────────────────────────────────────────────────
//

#[test]
fn point_order_blocked_at_final_approach_accepts_by_cell() {
    let mut app = utils::corner_app();
    // The keep occupies cells (8..=10, 6..=8); the ordered spot lies on the
    // runner's own cell, but standing on it exactly would poke the body into
    // the keep's west face — the exact spot is unreachable forever.
    utils::create_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::create_owned(&mut app, "runner", 7, 7, 0);

    app.world_mut()
        .entity_mut(runner)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(7.3), FixedU64::from_num(7)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 60);

    // The frustration ring accepts by cells: each walled attempt escalates
    // and the escalation must stand through the regain walk — forgiving it
    // there laundered the frustration every round-trip and ground forever.
    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must finish instead of grinding at the wall"
    );
    assert_eq!(utils::cell_of(app.world_mut(), runner), CellPos::new(7, 7));
}

#[test]
fn body_clipped_into_footprint_walks_itself_free() {
    let mut app = utils::corner_app();
    utils::create_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::create_owned(&mut app, "runner", 7, 7, 0);

    // A building raised against the body's edge leaves the circle clipping
    // the static footprint — staged directly, since placement reads claims at
    // the rounded anchor and cannot see circle edges. Deep enough that no
    // single step clears it, so draining is the only way out.
    app.world_mut()
        .entity_mut(runner)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = FixedUVec2::new(FixedU64::from_num(7.45), FixedU64::from_num(7));

    app.world_mut()
        .entity_mut(runner)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(4), FixedU64::from_num(7)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 300);

    assert!(
        app.world()
            .entity(runner)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the clipped body must walk itself free and finish"
    );
    assert_eq!(utils::cell_of(app.world_mut(), runner), CellPos::new(4, 7));
}

//
// ─── Yielding to a walk ────────────────────────────────────────────────────────
//

#[test]
fn wide_blocker_steps_aside_where_its_footprint_fits() {
    // A wall pins the lane from above and a stray block sits at (10, 6): the
    // step-aside must be measured with the blocker's own footprint and mask —
    // measured by single cells, the wagon is handed a spot its 2x2 body
    // cannot take (or backs into the walk's own face) and never clears the
    // way.
    let mut app = utils::cell_crowd_app();
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 6..=12 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
        }
        map.set_static_occupied(utils::GROUND, CellPos::new(10, 6), true);
    }
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 8, 5, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 6, 5, 0);

    app.world_mut()
        .entity_mut(soldier)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target: FixedUVec2::new(FixedU64::from_num(12), FixedU64::from_num(5)),
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );

    utils::run_ticks(&mut app, 200);

    assert_eq!(
        utils::cell_of(app.world_mut(), wagon),
        CellPos::new(8, 6),
        "the wagon stepped aside with its whole footprint"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(12, 5),
        "the walk passed through the vacated lane"
    );
}

//
// ─── Corridor walks ─────────────────────────────────────────────────────────
//

#[test]
fn cell_walk_follows_corridor_past_its_first_leg() {
    // The wall's gap is the only way across, so the walk is planned as a
    // corridor and refined one leg at a time. Running out of the current leg is
    // not arriving — the walk has to ask for the next one, or it stops at the
    // first crossing a couple of cells from where it started.
    let mut app = utils::orders_app();
    utils::install_choke_point_map(&mut app);
    let (soldier, id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(90, 90),
            flush: true,
        },
    );

    // Corner to corner by way of the gap is a long way round: roughly 120 cells
    // at the soldier's pace, with room to spare for the leg changes.
    utils::run_ticks(&mut app, utils::APPLY + 900);

    let arrived = utils::cell_of(app.world_mut(), soldier);
    assert_eq!(
        arrived,
        CellPos::new(90, 90),
        "the walk stopped at {arrived:?} instead of crossing the wall"
    );
}

//
// ─── Winding a walk down ────────────────────────────────────────────────────
//

#[test]
fn stop_ends_cell_walk_after_its_current_step() {
    // A soft cancel keeps only the step in progress and discards the rest of
    // the route. The walk then has to *end* when that step lands: an emptied
    // path looks exactly like a spent corridor leg, and a walk that reads it as
    // one plans the whole journey again and carries on to the destination the
    // player just canceled.
    let mut app = utils::orders_app();
    let (soldier, id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(28, 2),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 8);
    let before = utils::cell_of(app.world_mut(), soldier);

    utils::push_command(&mut app, PlayerCommand::Stop);
    utils::run_ticks(&mut app, utils::APPLY + 60);

    let after = utils::cell_of(app.world_mut(), soldier);
    assert!(
        after.x - before.x <= 2,
        "told to stop at {before:?}, the walk carried on to {after:?}"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), soldier),
        "the canceled walk is still queued"
    );
}

//
// ─── Coming round ───────────────────────────────────────────────────────────
//

/// A body that declares a pivot angle lines up before it walks: ordered back the
/// way it is looking, it spends ticks turning where it stands and sets off only
/// once its look is within a lean of the way it is going.
#[test]
fn ponderous_body_comes_round_before_it_walks() {
    let mut app = utils::turning_app();
    let (mover, _) = utils::create_owned(&mut app, "ponderous", 10, 10, 0);
    // A fresh body looks south, so due north is the longest turn there is: half a
    // circle at a degree a tick, less the lean it is released at, is a hundred and
    // fifty-eight ticks of standing still.
    walk_to_spot(&mut app, mover, utils::pos(10, 4));
    let start = utils::position_of(app.world_mut(), mover);

    utils::run_ticks(&mut app, 157);
    assert_eq!(
        utils::position_of(app.world_mut(), mover),
        start,
        "it must come round before it walks"
    );
    let looked = utils::facing_of(app.world(), mover);
    assert_ne!(looked, Facing::SOUTH, "and it must be coming round");
    assert_ne!(looked, Facing::NORTH, "without arriving early");

    utils::run_ticks(&mut app, 2);
    assert_ne!(
        utils::position_of(app.world_mut(), mover),
        start,
        "then the walk starts"
    );
}

/// A body with no pivot angle never holds still for a turn: it sets off on the
/// tick it was told to, and its look catches up as it goes. This is what keeps
/// infantry answering a click immediately.
#[test]
fn nimble_body_walks_while_its_look_catches_up() {
    let mut app = utils::turning_app();
    let (mover, _) = utils::create_owned(&mut app, "nimble", 10, 10, 0);
    walk_to_spot(&mut app, mover, utils::pos(10, 4));
    let start = utils::position_of(app.world_mut(), mover);

    utils::run_ticks(&mut app, 1);

    assert_ne!(
        utils::position_of(app.world_mut(), mover),
        start,
        "it must walk at once"
    );
    let looked = utils::facing_of(app.world(), mover);
    assert_ne!(looked, Facing::NORTH, "with its look still catching up");
    assert_ne!(looked, Facing::SOUTH, "but catching up");
}

/// The ticks a body spends coming round are not ticks of getting nowhere. Counted
/// against the stall clock they would read as a walk being crowded off its way:
/// the walk would escalate, grow its acceptance, and finish short of the spot it
/// was sent to.
#[test]
fn ticks_spent_coming_round_do_not_escalate_walk() {
    let mut app = utils::turning_app();
    let (mover, _) = utils::create_owned(&mut app, "ponderous", 10, 10, 0);
    // A hundred and fifty-eight ticks of turning, against a stall clock that
    // escalates every fifteen and gives the walk up after eight escalations.
    let spot = utils::pos(10, 4);
    walk_to_spot(&mut app, mover, spot);

    utils::run_ticks(&mut app, 400);

    assert!(
        app.world()
            .entity(mover)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must end"
    );
    assert_eq!(
        utils::position_of(app.world_mut(), mover),
        spot,
        "and by arriving: a walk that counted its turn as going nowhere gives up \
         where it stands instead"
    );
}

/// The cell model holds its crossing the same way: a body that must come round
/// first does not stake a claim on the cell it is crossing into, because the claim
/// *is* the crossing — one held while the body is still turning would reserve
/// ground it has not set off for.
#[test]
fn cell_walk_comes_round_before_it_claims() {
    let mut app = utils::turning_cell_app();
    let (mover, _) = utils::create_owned(&mut app, "ponderous", 10, 10, 0);
    let start = utils::position_of(app.world_mut(), mover);
    walk_to_spot(&mut app, mover, utils::pos(10, 4));

    utils::run_ticks(&mut app, 157);

    assert_eq!(
        utils::position_of(app.world_mut(), mover),
        start,
        "the crossing must wait for the turn"
    );
    assert!(
        !app.world()
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(10, 9)),
        "and the cell it will cross into must be unclaimed until it sets off"
    );

    utils::run_ticks(&mut app, 2);
    assert_ne!(
        utils::position_of(app.world_mut(), mover),
        start,
        "then it crosses"
    );
}

/// A body that finds its next cell free and stands to turn toward it has not set
/// off yet: the cell may be taken again before it does, and a walk that counted
/// the free cell as clear going would forgive its blockages every time and never
/// give up. Here each cell the body turns toward is walled the tick it starts
/// turning, the last one opened again, so the way is always one turn from clear.
#[test]
fn walk_gives_up_when_cell_ahead_is_taken_each_time_it_turns() {
    let mut app = utils::turning_cell_app();
    let (mover, _) = utils::create_owned(&mut app, "ponderous", 10, 10, 0);
    let start = utils::position_of(app.world_mut(), mover);
    walk_to_spot(&mut app, mover, utils::pos(10, 4));

    let mut walled: Option<CellPos> = None;
    for _ in 0..400 {
        utils::run_ticks(&mut app, 1);
        let turning_toward = app
            .world()
            .entity(mover)
            .get::<MoveComponent>()
            .filter(|movement| movement.pivoting)
            .and_then(|movement| movement.path.last().copied());
        if let Some(cell) = turning_toward
            && walled != Some(cell)
        {
            let mut map = app.world_mut().resource_mut::<Map>();
            if let Some(previous) = walled {
                map.set_static_occupied(utils::GROUND, previous, false);
            }
            map.set_static_occupied(utils::GROUND, cell, true);
            walled = Some(cell);
        }
    }

    assert!(
        app.world()
            .entity(mover)
            .get::<OrderQueueComponent>()
            .is_some_and(|queue| queue.0.is_empty()),
        "the walk must give up"
    );
    assert_eq!(
        utils::position_of(app.world_mut(), mover),
        start,
        "without ever having set off"
    );
}

//
// ─── Walks off every region ─────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a walker stands on an anchor in a region of its shape")]
fn walk_starting_where_its_shape_has_no_region_panics() {
    // A landlubber, its domain taking land alone, is moved onto the water at
    // the pool's edge, (10,9), where its shape has no region: no set-down puts
    // a walker there, and a walk from there is refused loudly.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &utils::water_pool(), |_, _| {});
    let (landlubber, _) = utils::create_owned(&mut app, "landlubber", 10, 7, 0);
    spawn::relocate(app.world_mut(), &[(landlubber, CellPos::new(10, 9))]);

    utils::order_move(&mut app, landlubber, 10, 4);
    utils::run_ticks(&mut app, 2);
}

//
// ─── Served movers ──────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "map 'test' serves no hierarchy for mover 'archer'")]
fn game_whose_map_serves_no_mover_does_not_start() {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    utils::register_orders_content(&mut app);
    // A map built before its content and never served it: starting the game
    // refuses it, before anything has run.
    ferrets_bevy_plugin::start_game(app.world_mut());
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────────
//

/// A keep at cells (8..=10, 6..=8) and a runner pushed off-lattice west of its
/// north-east corner, so a walk west along row 5 clips corner cell (10, 6): the
/// wall lies across the way rather than beside it, and row 5 itself is clear the
/// whole way. Pushed rather than placed, because placement reads claims at the
/// rounded anchor and cannot leave a body part way across a cell.
fn runner_clipping_keep_corner(app: &mut App) -> Entity {
    utils::create_owned(app, "keep", 8, 6, 0);
    let (runner, _) = utils::create_owned(app, "runner", 12, 5, 0);
    app.world_mut()
        .entity_mut(runner)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = utils::part_way("10.68", "5.08");
    runner
}

/// Orders `entity` to walk to the exact spot `target`, as a click on open ground
/// does.
fn walk_to_spot(app: &mut App, entity: Entity, target: FixedUVec2) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                target,
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );
}
