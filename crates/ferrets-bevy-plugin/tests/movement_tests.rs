//! Movement that the plan alone does not settle: a continuous body pressed into
//! a building corner must round it rather than converge on the corner's tangent
//! line forever and must look where it steps while doing it, and a cell walk
//! planned as a corridor must follow the corridor rather than stop where its
//! first leg ran out.

mod utils;

use bevy::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{location::LocationComponent, order_queue::OrderQueueComponent},
    map::Map,
    movement_model::MovementModel,
    order::Order,
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
    let mut looked = facing_of(&app, runner);
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
        let facing = facing_of(&app, runner);
        if travelled < crumb {
            crumbs += 1;
            assert_eq!(
                facing, looked,
                "tick {tick}: a crumb step of {step:?} turned the body"
            );
            continue;
        }
        // The whole rule, exactly: the look a step is worth seeing for *is* that
        // step.
        assert_eq!(facing, step, "tick {tick}: stepped one way, looked another");
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
    utils::spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 7, 7, 0);

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
    utils::spawn_owned(&mut app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 7, 7, 0);

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
    let (wagon, _) = utils::spawn_owned(&mut app, "wagon", 8, 5, 0);
    let (soldier, _) = utils::spawn_owned(&mut app, "soldier", 6, 5, 0);

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
    utils::install_chokepoint_map(&mut app);
    let (soldier, id) = utils::spawn_owned(&mut app, "soldier", 2, 2, 0);

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
    // player just cancelled.
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Cell);
    let (soldier, id) = utils::spawn_owned(&mut app, "soldier", 2, 2, 0);

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
        "the cancelled walk is still queued"
    );
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
    utils::spawn_owned(app, "keep", 8, 6, 0);
    let (runner, _) = utils::spawn_owned(app, "runner", 12, 5, 0);
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

/// The look the renderer draws `entity` at.
fn facing_of(app: &App, entity: Entity) -> FixedVec2 {
    app.world()
        .entity(entity)
        .get::<LocationComponent>()
        .unwrap()
        .facing
}
