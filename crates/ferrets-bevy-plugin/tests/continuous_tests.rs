//! The continuous movement model: free positions, bodies pushing apart, and
//! terrain-checked commits.
//!
//! Rest positions and cells are asserted exactly where they are pinned:
//! every value is deterministic fixed-point math replayed identically on
//! every lockstep peer, so a drifted equilibrium — however plausible — is
//! a desync. Changing a pinned value must be a conscious decision.

mod utils;

use bevy::prelude::{App, Entity};
use ferrets_content::{
    area::Area,
    entity_buffs::{EntityBuffDef, Lasting},
    entity_effect::EntityEffect,
    entity_type_def::EntityTypeDef,
    kinds::Kinds,
    location::Solidity,
    pool::Pool,
    quantity::Quantity,
    registry::ContentRegistry,
    relation::Relation,
    skills::{Casting, EntityCastEffect, Reach, Recipient, SkillCaster, SkillDef, Whom},
    stack_rule::StackRule,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
    shape::Shape,
};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{movement::MoveComponent, order_queue::OrderQueueComponent, stance::Stance},
    entity_def,
    map::Map,
    movement_model::MovementModel,
    order::{AttackTarget, MoveGoal, Order},
    session::{player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
};

#[test]
fn continuous_walk_reaches_goal() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (soldier, id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 14),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 200);

    assert_eq!(
        utils::anchor_of(app.world_mut(), soldier),
        CellPos::new(20, 14)
    );
    // The uncrowded walk lands exactly on the lattice: the final step hits
    // its waypoint to the bit.
    let world = app.world_mut();
    assert_eq!(utils::position_of(world, soldier), utils::pos(20, 14));
}

#[test]
fn overlapping_bodies_push_apart() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // Two soldiers ordered onto the same cell end up as separated bodies,
    // not stacked points, and both walks settle.
    let first = utils::create_owned(&mut app, "soldier", 3, 5, 0);
    let second = utils::create_owned(&mut app, "soldier", 7, 5, 0);
    for (_, id) in [first, second] {
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(5, 5),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 300);

    let world = app.world_mut();
    let a = utils::position_of(world, first.0);
    let b = utils::position_of(world, second.0);
    // The exact equilibrium: both walks touch the contested spot the same
    // tick and finish there; the pushing pass parts the coincident pair
    // half a body each way, then the idle pair drifts 0.125 more each way to
    // the 0.25 idle spacing: middles 1 + 0.25 = 1.25 apart.
    assert_eq!(a, utils::point("4.875", "5.5"));
    assert_eq!(b, utils::point("6.125", "5.5"));
}

#[test]
fn pushing_never_commits_onto_terrain_off_its_domain() {
    // The crowd of the test below, of landlubbers, against a column of water
    // rather than a wall: their domain takes land alone, so the water is the
    // same barrier, in planning and in every shove.
    let water: Vec<(u32, u32)> = (2..9).map(|y| (8, y)).collect();
    let mut app = utils::dry_orders_app(MovementModel::Continuous, &water, |_, _| {});

    let landlubbers: Vec<_> = [(2, 4), (3, 5), (2, 6), (4, 5), (3, 4)]
        .iter()
        .map(|&(x, y)| utils::create_owned(&mut app, "landlubber", x, y, 0))
        .collect();
    for (_, id) in &landlubbers {
        utils::select(&mut app, *id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(7, 5),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 300);

    // The settle of the wall test, cell for cell, and no circle over the
    // water.
    let world = app.world_mut();
    let cells: Vec<CellPos> = landlubbers
        .iter()
        .map(|(landlubber, _)| utils::anchor_of(world, *landlubber))
        .collect();
    assert_eq!(
        cells,
        vec![
            CellPos::new(6, 4),
            CellPos::new(6, 5),
            CellPos::new(6, 7),
            CellPos::new(7, 5),
            CellPos::new(7, 3),
        ]
    );
    // None over the water: the one on the spot rests flush against it, its
    // middle at x = 7.5 reaching to 7.5 + 0.5 = 8, the water's edge, and the
    // rest stand short of it around the spot.
    let edges: Vec<FixedU64> = landlubbers
        .iter()
        .map(|(landlubber, _)| utils::position_of(world, *landlubber).x)
        .collect();
    assert_eq!(
        edges,
        vec![
            utils::fixed("6.5170131943"),
            utils::fixed("6.2885074061"),
            utils::fixed("6.9964237784"),
            utils::fixed("7.5"),
            utils::fixed("7.0799790972"),
        ]
    );
}

#[test]
fn pushing_never_commits_into_walls() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A wall behind the meeting point: a crowd shoved against it must slide
    // along, never onto the blocked cells.
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for y in 2..9 {
            map.set_static_occupied(utils::GROUND, CellPos::new(8, y), true);
        }
    }

    let soldiers: Vec<_> = [(2, 4), (3, 5), (2, 6), (4, 5), (3, 4)]
        .iter()
        .map(|&(x, y)| utils::create_owned(&mut app, "soldier", x, y, 0))
        .collect();
    for (_, id) in &soldiers {
        utils::select(&mut app, *id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(7, 5),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 300);

    // The exact settle against the wall, in spawn order: a column one cell
    // short of it, spaced along it around the taken goal.
    {
        let world = app.world_mut();
        let cells: Vec<CellPos> = soldiers
            .iter()
            .map(|(soldier, _)| utils::anchor_of(world, *soldier))
            .collect();
        assert_eq!(
            cells,
            vec![
                CellPos::new(6, 4),
                CellPos::new(6, 5),
                CellPos::new(6, 7),
                CellPos::new(7, 5),
                CellPos::new(7, 3),
            ]
        );
    }
    // No body's circle may overlap the wall: for every bounding cell the
    // circle actually reaches into (strictly past its nearest point), the
    // cell must be statically passable.
    for (soldier, _) in soldiers {
        let world = app.world_mut();
        let position = utils::position_of(world, soldier);
        let half = utils::fixed("0.5");
        let middle = (position.x, position.y);
        let low = (position.x - half, position.y - half);
        let radius = half;
        let cells_of = |value: FixedU64| {
            let first = value.floor().to_num::<u32>();
            if value.frac() == FixedU64::ZERO {
                first..=first
            } else {
                first..=first + 1
            }
        };
        for y in cells_of(low.1) {
            for x in cells_of(low.0) {
                let nearest = |value: FixedU64, cell: u32| {
                    value.clamp(FixedU64::from_num(cell), FixedU64::from_num(cell + 1))
                };
                let off_x = middle.0.abs_diff(nearest(middle.0, x)).to_bits() as u128;
                let off_y = middle.1.abs_diff(nearest(middle.1, y)).to_bits() as u128;
                let reach = radius.to_bits() as u128;
                if off_x * off_x + off_y * off_y >= reach * reach {
                    continue;
                }
                assert!(
                    !world
                        .resource::<Map>()
                        .nav_grid()
                        .is_statically_occupied(utils::GROUND, CellPos::new(x, y)),
                    "body at {position:?} overlaps wall cell ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn head_on_bodies_flow_past_each_other() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // Exactly aligned on one row — the lattice's common case. Radial-only
    // pushing stalls or tunnels here; the sideways share must carry both
    // around and through to their targets in near-direct time.
    let (left, left_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    let (right, right_id) = utils::create_owned(&mut app, "soldier", 9, 5, 0);

    utils::select(&mut app, left_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 5),
            flush: true,
        },
    );
    utils::select(&mut app, right_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(2, 5),
            flush: true,
        },
    );

    // Sample the pair every tick: bodies must slide around each other, so
    // they never interpenetrate — tunneling straight through would dip the
    // distance far below a body diameter.
    let mut min_distance = FixedU64::MAX;
    for _ in 0..100 {
        utils::run_ticks(&mut app, 1);
        let world = app.world_mut();
        let (a, b) = (
            utils::position_of(world, left),
            utils::position_of(world, right),
        );
        let distance = a.distance(b);
        min_distance = min_distance.min(distance);
    }

    assert_eq!(utils::anchor_of(app.world_mut(), left), CellPos::new(9, 5));
    assert_eq!(utils::anchor_of(app.world_mut(), right), CellPos::new(2, 5));
    // The pair never comes a single bit closer than a full body diameter:
    // the swerve parts them exactly at contact.
    assert_eq!(min_distance, FixedU64::ONE);
}

#[test]
fn crowded_group_settles_and_stops_milling() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // Everyone ordered onto the same point cannot all take it: the walks
    // must finish in a ring around it (not grind forever), and the settled
    // pile must come to genuine rest — sustained contact must not keep
    // churning bodies around each other.
    let soldiers: Vec<_> = [(2, 2), (10, 2), (2, 8), (10, 8), (6, 1), (6, 9)]
        .iter()
        .map(|&(x, y)| utils::create_owned(&mut app, "soldier", x, y, 0))
        .collect();
    for (_, id) in &soldiers {
        utils::select(&mut app, *id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(6, 5),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 600);

    let world = app.world_mut();
    for (soldier, _) in &soldiers {
        assert!(
            utils::order_queue_is_empty(world, *soldier),
            "every walk into the crowd must finish"
        );
    }

    let positions = |app: &mut bevy::prelude::App| -> Vec<_> {
        soldiers
            .iter()
            .map(|(soldier, _)| utils::position_of(app.world_mut(), *soldier))
            .collect()
    };
    let settled = positions(&mut app);
    utils::run_ticks(&mut app, 50);
    assert_eq!(
        settled,
        positions(&mut app),
        "a settled crowd must rest, not mill around"
    );

    // The exact settle around the contested point: the idle spacing apart
    // where a body had room to drift, touching where it was squeezed.
    let mut cells: Vec<CellPos> = soldiers
        .iter()
        .map(|(soldier, _)| utils::anchor_of(app.world(), *soldier))
        .collect();
    cells.sort_unstable();
    assert_eq!(
        cells,
        vec![
            CellPos::new(4, 5),
            CellPos::new(5, 4),
            CellPos::new(5, 6),
            CellPos::new(7, 4),
            CellPos::new(7, 6),
            CellPos::new(8, 5),
        ]
    );
}

#[test]
fn idle_crowd_rests_its_spacing_apart() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A packed double column marching onto one point: once every walk has
    // ended the crowd is idle, and drifts until every two circles lie the idle
    // spacing, 0.25 cells, apart, within its slack of 1/64, then rests.
    let soldiers: Vec<_> = (0..10)
        .map(|i| utils::create_owned(&mut app, "soldier", 2 + i % 2, 2 + i / 2, 0))
        .collect();
    for (_, id) in &soldiers {
        utils::select(&mut app, *id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(12, 4),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    for (soldier, _) in &soldiers {
        assert!(
            utils::order_queue_is_empty(world, *soldier),
            "every walk into the crowd must finish"
        );
    }
    let positions = |app: &mut App| -> Vec<_> {
        soldiers
            .iter()
            .map(|(soldier, _)| utils::position_of(app.world_mut(), *soldier))
            .collect()
    };
    let settled = positions(&mut app);
    utils::run_ticks(&mut app, 50);
    assert_eq!(settled, positions(&mut app), "a spaced crowd rests");
    let world = app.world();
    let nearest = soldiers
        .iter()
        .enumerate()
        .flat_map(|(index, &(first, _))| {
            soldiers[index + 1..].iter().map(move |&(second, _)| {
                entity_def::shape(world, first).distance(entity_def::shape(world, second))
            })
        })
        .min()
        .expect("ten soldiers make pairs");
    // Within the slack of the spacing: 0.25 − 1/64 = 0.234375 ≤ 0.2369 ≤ 0.25.
    assert_eq!(nearest, utils::fixed("0.2368606948"));
}

/// An idle crowd walled in tighter than the spacing allows rests rather
/// than jittering: nine soldiers, one a cell in a 3×3 pocket, every other
/// one nudged 0.2 off its cell, stand still once settled.
#[test]
fn idle_crowd_boxed_too_tight_rests() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 1..=5 {
            for y in 1..=5 {
                if x == 1 || x == 5 || y == 1 || y == 5 {
                    map.set_static_occupied(utils::GROUND, CellPos::new(x, y), true);
                }
            }
        }
    }
    let mut soldiers = Vec::new();
    for x in 2..=4 {
        for y in 2..=4 {
            soldiers.push(utils::create_owned(&mut app, "soldier", x, y, 0).0);
        }
    }
    for &soldier in soldiers.iter().step_by(2) {
        let at = utils::position_of(app.world_mut(), soldier);
        utils::move_body(
            &mut app,
            soldier,
            FixedUVec2::new(at.x + utils::fixed("0.2"), at.y),
        );
    }
    let positions = |app: &mut App| -> Vec<_> {
        soldiers
            .iter()
            .map(|&soldier| utils::position_of(app.world_mut(), soldier))
            .collect()
    };

    utils::run_ticks(&mut app, 100);
    let settled = positions(&mut app);
    utils::run_ticks(&mut app, 20);

    assert_eq!(settled, positions(&mut app), "a boxed crowd rests");
}

#[test]
fn pushed_idle_body_claims_cell_it_settles_on() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A one-cell corridor, so every shove lands along the row:
    //
    //   ########
    //   .W..I...      W: walker (2,5) → (11,5)    I: idle body (6,5)
    //   ########
    //
    // The walker cannot pass a body in a corridor its own width — it plows
    // the idle unit ahead of itself. Occupancy must follow the body: the
    // cell it was shoved off is released, the cell it settles on is claimed.
    {
        let world = app.world_mut();
        let mut map = world.resource_mut::<Map>();
        for x in 1..13 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 6), true);
        }
    }
    let (idle, _) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    let cell = utils::anchor_of(world, idle);
    assert!(
        cell.x > 6 && cell.y == 5,
        "idle body must be plowed down the corridor: rests on {cell:?}"
    );
    // The exact end state: the idle body plowed past the walker's spot, the
    // walk ending once the walker stands as near the spot as the body lets
    // it, the two then drifting apart to the idle spacing.
    assert_eq!(utils::anchor_of(world, idle), CellPos::new(11, 5));
    assert_eq!(utils::anchor_of(world, walker), CellPos::new(10, 5));
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_claimed(utils::GROUND, CellPos::new(6, 5)),
        "pushed body must release the cell it was shoved off"
    );

    // The claim plane holds one cell per body — the cell under its middle,
    // the one it visually stands on: every corridor cell is claimed exactly
    // when it is some body's middle cell, however the bodies straddle
    // borders.
    let bodies: Vec<CellPos> = [idle, walker]
        .iter()
        .map(|&entity| utils::anchor_of(world, entity))
        .collect();
    for x in 1..13 {
        let cell = CellPos::new(x, 5);
        let expected = bodies.contains(&cell);
        assert_eq!(
            world
                .resource::<Map>()
                .nav_grid()
                .is_claimed(utils::GROUND, cell),
            expected,
            "claim on {cell:?} must be the cell under a body's middle"
        );
    }
}

#[test]
fn wide_body_claims_whole_footprint() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A resting 2x2 body claims all four cells of its footprint, not one
    // cell under its anchor.
    let _ = utils::create_owned(&mut app, "wagon", 5, 5, 0);

    let world = app.world_mut();
    let grid = world.resource::<Map>().nav_grid();
    for (x, y) in [(5, 5), (6, 5), (5, 6), (6, 6)] {
        assert!(
            grid.is_claimed(utils::GROUND, CellPos::new(x, y)),
            "the footprint cell ({x}, {y}) is unclaimed"
        );
    }
    for (x, y) in [(4, 5), (7, 5), (5, 4), (5, 7)] {
        assert!(
            !grid.is_claimed(utils::GROUND, CellPos::new(x, y)),
            "the neighbor cell ({x}, {y}) is claimed past the footprint"
        );
    }
}

#[test]
fn push_never_sets_footprint_past_map_edge() {
    // A slim wagon — its circle of radius 0.9 inside its ⌈2 × 0.9⌉ = 2-cell
    // square body — rests against the east edge of the 32-cell map, anchored
    // at (30, 5), its middle at (31, 6). A soldier set inside its north-west
    // quarter pushes it south-east, where its circle would still sit on the
    // map but its body would reach past the edge: the push's east share is
    // held at x 32 − 2/2 = 31, and its south share slides the wagon along the
    // edge, its footprint still ending on column 31; a drift that would slide
    // it is not taken.
    let mut app = slim_wagon_app();
    let (wagon, _) = utils::create_owned(&mut app, "slim_wagon", 30, 5, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 9, 9, 0);
    utils::move_body(&mut app, soldier, utils::pos(30, 5));

    utils::run_ticks(&mut app, 300);

    // Its middle held at x 31 and slid south along the edge from y 6 to
    // 6.2450, the parting's south shares pinned to the bit; its anchor still
    // (30, 5), since 6.2450 − 2/2 + 0.5 floors to 5.
    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, wagon),
        utils::position_bits(0x1f_0000_0000, 0x6_3eb6_aa3e)
    );
    assert_eq!(utils::anchor_of(world, wagon), CellPos::new(30, 5));
}

#[test]
fn continuous_claim_form_counts_held_cells_not_snapped_ones() {
    // A continuous 2×2 wagon anchored at (9, 9) has its middle at (10, 10).
    // Judged as a one-cell soldier, a claim form a continuous body holds as
    // it is: its low corner 10 − 1/2 = 9.5 rounds up, so it counts (10, 10)
    // — not (9, 9), which snapping the tie toward the wagon's own anchor
    // would give.
    let mut app = utils::continuous_orders_app();
    let (wagon, _) =
        utils::create_entity(app.world_mut(), "wagon", utils::cell(9, 9), Some(0)).unwrap();

    let world = app.world();
    let soldier = world
        .resource::<ContentRegistry>()
        .entity("soldier")
        .expect("the orders content declares a soldier");
    assert_eq!(
        entity_def::footprint_as(world, wagon, soldier),
        CellRect::cell(CellPos::new(10, 10))
    );
}

//
// ─── Map edges ────────────────────────────────────────────────────────────────
//

#[test]
fn point_move_before_origin_holds_body_on_map() {
    // A soldier ordered to (0.2, 3.3) would reach 0.2 − 1/2 before the
    // origin: the walk aims at x 1/2 instead and finishes there, y kept.
    let mut app = utils::continuous_orders_app();
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 3, 0);
    utils::order_move_onto(&mut app, soldier, utils::point("0.2", "3.3"));

    utils::run_ticks(&mut app, 200);

    assert!(utils::order_queue_is_empty(app.world(), soldier));
    assert_eq!(
        utils::position_of(app.world(), soldier),
        utils::point("0.5", "3.3")
    );
}

#[test]
fn wide_point_move_past_either_edge_holds_body_on_map() {
    // A slim wagon — its circle of radius 0.9, its body ⌈2 × 0.9⌉ = 2 cells a
    // side — fits by its circle with its middle 0.9 from an edge, but its
    // body reaches a whole cell either side: ordered to x 0.5 it finishes at
    // 2/2 = 1, and to x 31.8 on the 32-wide map at 32 − 1 = 31.
    for (ordered, rests) in [("0.5", "1"), ("31.8", "31")] {
        let mut app = slim_wagon_app();
        let (wagon, _) = utils::create_owned(&mut app, "slim_wagon", 15, 5, 0);
        utils::order_move_onto(&mut app, wagon, utils::point(ordered, "6"));

        utils::run_ticks(&mut app, 400);

        assert!(
            utils::order_queue_is_empty(app.world(), wagon),
            "ordered to x {ordered}"
        );
        assert_eq!(
            utils::position_of(app.world(), wagon),
            utils::point(rests, "6"),
            "ordered to x {ordered}"
        );
    }
}

#[test]
fn mixed_size_bodies_push_apart() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A soldier spawned inside a resting wagon's footprint: contact runs
    // middle to middle — the wagon's sits half its size past its anchor's
    // corner, deeper in than a same-anchor small body's — so the pair parts
    // by the sum of their radii instead of missing the overlap or phantom
    // pushing on one side.
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 5, 5, 0);
    // Spawned clear — placement rightly refuses the claimed footprint — and
    // moved into the overlap directly, its claim following it.
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 9, 9, 0);
    utils::move_body(&mut app, soldier, utils::pos(6, 6));

    utils::run_ticks(&mut app, 300);

    let world = app.world_mut();
    // The exact equilibrium: the pair parts along the diagonal through both
    // middles, each carried half the overlap a pass until contact clears,
    // then drifts the same way until the circles lie the idle spacing apart —
    // middles 1 + 0.5 + 0.25 = 1.75 apart, less the diagonal's rounding (the
    // overlap and the drift past it rounded apart).
    assert_eq!(
        utils::position_of(world, wagon),
        utils::position_bits(0x5_a19b_ab34, 0x5_a19b_ab34)
    );
    assert_eq!(
        utils::position_of(world, soldier),
        utils::position_bits(0x6_de64_54cc, 0x6_de64_54cc)
    );
}

/// Weight is authored apart from size: an ox fills the one cell a soldier
/// does and weighs four times as much, so the contact between them carries
/// the soldier the greater part of the way.
#[test]
fn heavier_body_is_carried_less_than_lighter_of_same_size() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (ox, _) = utils::create_owned(&mut app, "ox", 5, 5, 0);
    // Spawned clear — placement rightly refuses the claimed cell — then set
    // half a cell into the ox, half the radii sum of the two bodies.
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 9, 9, 0);
    utils::move_body(&mut app, soldier, utils::point("6", "5.5"));

    utils::run_ticks(&mut app, 300);

    let world = app.world_mut();
    let ox_gave = utils::fixed("5.5") - utils::position_of(world, ox).x;
    let soldier_gave = utils::position_of(world, soldier).x - FixedU64::from_num(6);
    assert!(
        soldier_gave > ox_gave * FixedU64::from_num(3),
        "the soldier must give far more ground than the ox it met: \
         soldier {soldier_gave}, ox {ox_gave}"
    );
}

#[test]
fn wide_walk_reaches_goal() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (wagon, id) = utils::create_owned(&mut app, "wagon", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 14),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 300);

    // An uncrowded wide walk lands its anchor exactly on the ordered cell,
    // like the single-cell walk does — paths are anchor sequences at every
    // size.
    let world = app.world_mut();
    assert_eq!(utils::position_of(world, wagon), utils::pos(20, 14));
}

#[test]
fn claim_follows_walking_body() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (soldier, id) = utils::create_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 2),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 200);

    // The claim follows the body: the cell under it is claimed, the spawn
    // cell released.
    let world = app.world_mut();
    let cell = utils::anchor_of(world, soldier);
    assert_eq!(cell, CellPos::new(12, 2));
    assert!(
        world
            .resource::<Map>()
            .nav_grid()
            .is_claimed(utils::GROUND, cell)
    );
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_claimed(utils::GROUND, CellPos::new(2, 2))
    );
}

/// A walk within a range whose goal cell a body that is not walking stands on
/// — one too heavy to shove, pinned between walls — routes at once to another
/// cell that serves the goal when its stall clock escalates, as the cell
/// model's rung does, rather than pressing on it for good.
#[test]
fn attacker_takes_free_cell_beside_body_holding_its_goal() {
    let mut app = utils::continuous_orders_app();
    let (rampart, rampart_id) = utils::create_owned(&mut app, "rampart", 10, 10, 1);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [(8, 9), (9, 9), (8, 11), (9, 11)] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), true);
        }
    }
    utils::create_owned(&mut app, "ox", 9, 10, 0);
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 3, 10, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(rampart_id),
            flush: true,
        },
    );

    // The walk meets the ox at (9,10), which stands on a waypoint of its
    // route; the waypoint is dropped and the stalled walk routes around the
    // walls to (10,9) above the rampart, where its first blow of 10 lands on
    // the forty-fifth tick.
    utils::run_ticks(&mut app, 44);
    assert_eq!(utils::health_as_u32(&app, rampart), 400);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, rampart), 390);
    assert_eq!(
        utils::position_of(app.world_mut(), soldier),
        utils::point("10.5", "9")
    );
}

/// The first distance after a route taken around a body holding the goal
/// forgives nothing: the same attacker, escalated once when it takes the
/// route, keeps its rung until its first pop on the route lands.
#[test]
fn first_distance_on_taken_route_forgives_nothing() {
    let mut app = utils::continuous_orders_app();
    let (_, rampart_id) = utils::create_owned(&mut app, "rampart", 10, 10, 1);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [(8, 9), (9, 9), (8, 11), (9, 11)] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), true);
        }
    }
    utils::create_owned(&mut app, "ox", 9, 10, 0);
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 3, 10, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(rampart_id),
            flush: true,
        },
    );
    let frustration = |app: &App| {
        app.world()
            .get::<MoveComponent>(soldier)
            .expect("the attacker walks")
            .frustration
    };

    // Stalled on the twenty-ninth tick, the walk escalates to 1 and takes
    // the route; the thirtieth tick's first distance is a baseline and the
    // step pops (7, 10); the pop lands on the thirty-first, forgiving.
    utils::run_ticks(&mut app, 30);
    let on_baseline = frustration(&app);
    utils::run_ticks(&mut app, 1);
    assert_eq!((on_baseline, frustration(&app)), (1, 0));
}

/// A point walk behind a column of idle allies pinned against a wall, unable
/// to pass them, gives up once the spot is taken: a step that only presses
/// into the first of them is no progress, so its stall clock escalates, its
/// acceptance widens, and it ends on finding a resting ally within it, three
/// cells short of its spot.
#[test]
fn walk_behind_pinned_allies_ends_once_spot_taken() {
    let (mut app, soldier, oxen) = oxen_in_corridor(0);

    // Stalled behind the first ox from the ninth tick, the walk escalates
    // fifteen ticks on, on the twenty-fourth — its detour's first cell
    // reached by a step inside the first ox, which hands it back past the pop
    // slack, so nothing is forgiven, and the first distance after it only
    // sets a record — and again on the fortieth, ending there, anchored at
    // (6, 10). The oxen hold their column at x 7.5, 8.5 and 9.5, the first
    // two 2 and 1 bits east of it, contact's rounding left from their drift.
    utils::run_ticks(&mut app, utils::APPLY + 39);
    assert!(!utils::order_queue_is_empty(app.world_mut(), soldier));
    utils::run_ticks(&mut app, 1);
    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, soldier));
    assert_eq!(utils::anchor_of(world, soldier), CellPos::new(6, 10));
    let column: Vec<_> = oxen
        .iter()
        .map(|&ox| utils::position_of(world, ox))
        .collect();
    assert_eq!(
        column,
        vec![
            utils::position_bits(0x7_8000_0002, 0xa_8000_0000),
            utils::position_bits(0x8_8000_0001, 0xa_8000_0000),
            utils::point("9.5", "10.5")
        ]
    );
}

/// The same behind resting enemies: they block the corridor and do not give
/// way, and no resting ally lies within the widened acceptance, so the walk
/// presses on past its stall.
#[test]
fn walk_stalled_behind_resting_enemies_presses_on() {
    let (mut app, soldier, _) = oxen_in_corridor(1);

    utils::run_ticks(&mut app, utils::APPLY + 88);
    assert!(!utils::order_queue_is_empty(app.world_mut(), soldier));
    // Anchored at (6, 10), three cells short of (9, 10), eighty-eight ticks
    // on: the oxen are no resting allies, so the widened acceptance never
    // ends the walk.
    assert_eq!(
        utils::anchor_of(app.world_mut(), soldier),
        CellPos::new(6, 10)
    );
}

/// The same behind enemies with an ally resting just past the widened
/// acceptance: an ally out of range of the spot does not end the walk.
#[test]
fn walk_stalled_with_ally_resting_past_acceptance_presses_on() {
    let (mut app, soldier, _) = oxen_in_corridor(1);
    // An idle ally at (9, 12), behind the corridor's wall, lies 2 cells from
    // (9, 10), past the acceptance of 1 the stall widens to.
    let (ally, _) = utils::create_owned(&mut app, "ox", 9, 12, 0);

    utils::run_ticks(&mut app, utils::APPLY + 88);
    let world = app.world_mut();
    assert!(!utils::order_queue_is_empty(world, soldier));
    // Still short of (9, 10), stopped by the enemy ox at (7, 10), eighty-eight
    // ticks on, the ally resting where it was set down.
    assert_eq!(utils::anchor_of(world, soldier), CellPos::new(6, 10));
    assert_eq!(utils::anchor_of(world, ally), CellPos::new(9, 12));
}

/// A pop forgives by where contact leaves the walker, whatever order the
/// walkers step in: two soldiers stalled in a corridor behind an enemy, one
/// behind the other, walk on once it dies; the rear one steps into the front
/// one before it has moved, yet stands where it popped after contact, so both
/// are forgiven, created front first or rear first.
#[test]
fn column_walking_on_after_stall_is_forgiven_in_either_order() {
    for created in [Created::FrontFirst, Created::RearFirst] {
        let (mut app, column) = column_behind_enemy(created);
        utils::run_ticks(&mut app, 45);
        let stalled = frustrations(&app, &column);
        let (blocker, _) = column.blocker;
        utils::despawn_killed(app.world_mut(), blocker, column.killer, Some(0));
        utils::run_ticks(&mut app, 4);

        // Stalled at (7.5, 10.5) and (6.5, 10.5), touching, both escalate
        // twice by the forty-fifth tick (fifteen ticks a rung). Once the ox
        // dies each pops its next waypoint on the third tick, 0.5 a step,
        // and stands on it after contact: forgiven on the fourth.
        assert_eq!(
            (stalled, frustrations(&app, &column)),
            ((2, 2), (0, 0)),
            "created: {created:?}"
        );
    }
}

/// A walk's progress is measured toward where it aims: a body at 0.03 a tick
/// ordered to (9.95, 5.95), off its goal cell's middle, walks the last leg
/// while drawing away from that middle, and still ends exactly on the spot.
#[test]
fn slow_walk_to_spot_off_cell_middle_ends_on_it() {
    let mut app = paced_app(CellSize::ONE, "0.5", "0.03");
    let (walker, _) = utils::create_owned(&mut app, "bodied", 8, 5, 0);
    let spot = utils::point("9.95", "5.95");
    app.world_mut()
        .entity_mut(walker)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Move {
                goal: MoveGoal::Position(spot),
            },
            None,
        );

    let ended = (1..200).find(|_| {
        utils::run_ticks(&mut app, 1);
        utils::order_queue_is_empty(app.world_mut(), walker)
    });

    // From (8.5, 5.5), (1.45, 0.45) at 0.03 a tick under the isometric
    // projection's step: the forty-ninth tick, on the spot. Measured against
    // (9.5, 5.5) instead, the distance stops falling ≈ 0.955 along and rises
    // for ≈ 19 ticks — the stall clock would escalate on the fifteenth and the
    // ring end the walk short of the spot.
    assert_eq!(
        (ended, utils::position_of(app.world_mut(), walker)),
        (Some(49), spot)
    );
}

//
// ─── Bearing ──────────────────────────────────────────────────────────────────
//

/// Two touching bodies standing their ground take no idle spacing: both keep
/// the middles they were placed on, 1 apart, where an idle pair drifts to
/// 1.25.
#[test]
fn stand_ground_bodies_touching_keep_their_places() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // Spawned apart, so no pass parts them before the stance lands, then set
    // touching.
    let first = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let second = utils::create_owned(&mut app, "soldier", 9, 5, 0);
    for (_, id) in [first, second] {
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::SetStance {
                stance: Stance::StandGround,
            },
        );
    }
    utils::run_ticks(&mut app, 5);
    utils::move_body(&mut app, second.0, utils::point("6.5", "5.5"));

    utils::run_ticks(&mut app, 50);

    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, first.0),
        utils::point("5.5", "5.5")
    );
    assert_eq!(
        utils::position_of(world, second.0),
        utils::point("6.5", "5.5")
    );
}

/// A walker cannot plow a body standing its ground down a corridor its own
/// width, as it plows an idle one: the planted body keeps its middle.
#[test]
fn stand_ground_body_is_not_pushed_by_walker() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    walled_row(&mut app);
    let (planted, planted_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    // The walker stalls touching it: middles 0.5 + 0.5 = 1 apart.
    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, planted),
        utils::point("6.5", "5.5")
    );
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("5.5", "5.5")
    );
}

/// A walker whose way runs through a body standing its ground slides around
/// its rim and never steps into it: every tick the two middles lie at least
/// 0.5 + 0.5 = 1 apart, and the walk ends on its spot.
#[test]
fn walker_slides_around_stand_ground_body_on_its_way() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (planted, planted_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, 5);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 6),
            flush: true,
        },
    );
    let mut walked = Vec::new();
    for _ in 0..60 {
        utils::run_ticks(&mut app, 1);
        let world = app.world_mut();
        let apart = utils::position_of(world, walker).distance(utils::position_of(world, planted));
        assert!(
            apart >= FixedU64::ONE,
            "the walker stepped into it: {apart}"
        );
        walked.push(utils::position_of(world, walker));
    }

    // Routed toward (10, 6), the walk meets the body off its middle: on the
    // eighth tick its step touches the body and slides along its tangent,
    // 1.0221 from its middle. Walking in and being shoved back out would
    // leave it hugging the body at (5.8027, 6.2216), 1.0035 away.
    assert_eq!(walked[8], utils::point("5.6582840232", "6.0798816562"));
    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, walker));
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("10.5", "6.5")
    );
    assert_eq!(
        utils::position_of(world, planted),
        utils::point("6.5", "5.5")
    );
}

/// A point move onto the spot of a body standing its ground arrives on
/// meeting it, at its rim, and stands there.
#[test]
fn point_move_onto_stand_ground_body_arrives_at_its_rim() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (planted, planted_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, 5);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(6, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);

    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, walker));
    // It stops where it first touches the planted body, (5.5, 5.5), 1 from its
    // middle, and the walk ends there: the spot is taken.
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("5.5", "5.5")
    );
    assert_eq!(
        utils::position_of(world, planted),
        utils::point("6.5", "5.5")
    );
}

/// A disabled body takes no step of its own: touching an idle ally, neither
/// drifts, where an idle pair drifts to 1.25 apart.
#[test]
fn disabled_body_takes_no_idle_spacing() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (disabled, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (ally, _) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    utils::disable_for(&mut app, disabled, 1000);

    utils::run_ticks(&mut app, 50);

    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, disabled),
        utils::point("5.5", "5.5")
    );
    assert_eq!(utils::position_of(world, ally), utils::point("6.5", "5.5"));
}

/// A disabled body is pushable whatever its stance: a walker plows a disabled
/// body standing its ground down a corridor its own width, as it plows an
/// idle one.
#[test]
fn disabled_stand_ground_body_is_pushed_by_walker() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    walled_row(&mut app);
    let (disabled, disabled_id) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (_, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, disabled_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, 5);
    utils::disable_for(&mut app, disabled, 1000);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    // Plowed down the corridor until the walker stands as near its spot
    // (11.5, 5.5) as the body lets it — touching it, no farther from the spot
    // than touching a body on it — where the walk ends, as for a disabled body
    // on another stance.
    assert_eq!(
        utils::position_of(world, disabled),
        utils::point("11.25", "5.5")
    );
}

/// Disabled is not planted: a walker plows a disabled body down a corridor
/// its own width as it plows an idle one.
#[test]
fn disabled_body_is_pushed_by_walker() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    walled_row(&mut app);
    let (disabled, _) = utils::create_owned(&mut app, "soldier", 6, 5, 0);
    let (_, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::disable_for(&mut app, disabled, 1000);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    // Plowed down the corridor until the walker stands as near its spot
    // (11.5, 5.5) as the body lets it — touching it, no farther from the spot
    // than touching a body on it — where the walk ends, as for an idle body.
    assert_eq!(
        utils::position_of(world, disabled),
        utils::point("11.25", "5.5")
    );
}

//
// ─── Enemies ──────────────────────────────────────────────────────────────────
//

/// A walker cannot shove an idle enemy down a corridor its own width: it
/// stalls against it, the enemy keeping its middle.
#[test]
fn walker_stalls_against_idle_enemy_it_cannot_shove() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (enemy, _) = utils::create_owned(&mut app, "ox", 6, 5, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    assert_eq!(utils::position_of(world, enemy), utils::point("6.5", "5.5"));
    // The walker stalls touching it: middles 0.5 + 0.5 = 1 apart.
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("5.5", "5.5")
    );
}

/// A walker whose way runs through an idle enemy slides around its rim and
/// never steps into it: every tick the two middles lie at least 1 apart.
#[test]
fn walker_slides_around_idle_enemy_on_its_way() {
    let mut app = utils::continuous_orders_app();
    let (enemy, _) = utils::create_owned(&mut app, "ox", 6, 5, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(10, 6),
            flush: true,
        },
    );
    let mut walked = Vec::new();
    for _ in 0..60 {
        utils::run_ticks(&mut app, 1);
        let world = app.world_mut();
        let apart = utils::position_of(world, walker).distance(utils::position_of(world, enemy));
        assert!(
            apart >= FixedU64::ONE,
            "the walker stepped into it: {apart}"
        );
        walked.push(utils::position_of(world, walker));
    }

    // Routed toward (10, 6), the walk meets the body off its middle: on the
    // eighth tick its step touches the body and slides along its tangent,
    // 1.0221 from its middle. Walking in and being shoved back out would
    // leave it hugging the body at (5.8027, 6.2216), 1.0035 away.
    assert_eq!(walked[8], utils::point("5.6582840232", "6.0798816562"));
    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, walker));
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("10.5", "6.5")
    );
    assert_eq!(utils::position_of(world, enemy), utils::point("6.5", "5.5"));
}

/// A body standing its ground outranks the enemy meeting: an idle enemy set
/// 0.2 into it takes the whole overlap, 6.3 + 0.2 = 6.5, where two standing
/// enemies would halve it.
#[test]
fn idle_enemy_takes_whole_overlap_from_stand_ground_body() {
    let mut app = utils::continuous_orders_app();
    let (planted, planted_id) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (rival, _) = utils::create_owned(&mut app, "ox", 9, 9, 1);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, 5);
    utils::move_body(&mut app, rival, utils::point("6.3", "5.5"));

    utils::run_ticks(&mut app, 3);

    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, planted),
        utils::point("5.5", "5.5")
    );
    assert_eq!(utils::position_of(world, rival), utils::point("6.5", "5.5"));
}

/// A push never carries a body into one that cannot make way: in a one-cell
/// corridor an own ox set 0.6 into a disabled ally that touches an idle
/// enemy — the ally shut toward the enemy — takes the whole overlap back,
/// so the enemy keeps its place and the ally never comes nearer it than
/// touching, 0.5 + 0.5 = 1.
#[test]
fn push_never_carries_body_into_enemy() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (enemy, _) = utils::create_owned(&mut app, "ox", 8, 5, 1);
    let (ally, _) = utils::create_owned(&mut app, "ox", 7, 5, 0);
    let (shover, _) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::disable_for(&mut app, ally, 1000);
    utils::move_body(&mut app, shover, utils::point("6.9", "5.5"));

    let mut closest = FixedU64::MAX;
    for _ in 0..10 {
        utils::run_ticks(&mut app, 1);
        let world = app.world_mut();
        closest =
            closest.min(utils::position_of(world, ally).distance(utils::position_of(world, enemy)));
    }

    // Never nearer than touching, 0.5 + 0.5 = 1, at the end of a tick.
    assert_eq!(closest, FixedU64::ONE);
    let world = app.world_mut();
    assert_eq!(utils::position_of(world, enemy), utils::point("8.5", "5.5"));
    assert_eq!(utils::position_of(world, ally), utils::point("7.5", "5.5"));
    // The ally shut, the ox it pressed was pushed back out of it whole:
    // touching, 1 apart.
    assert_eq!(
        utils::position_of(world, shover),
        utils::point("6.5", "5.5")
    );
}

/// The same against a body standing its ground: the planted body keeps its
/// place, the ally stays touching it, and the ox pressing the ally is pushed
/// back out whole.
#[test]
fn push_never_carries_body_into_stand_ground_body() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (planted, planted_id) = utils::create_owned(&mut app, "ox", 8, 5, 0);
    let (ally, _) = utils::create_owned(&mut app, "ox", 7, 5, 0);
    let (shover, _) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::disable_for(&mut app, ally, 1000);
    // Set back where they were made: idle for the ticks the stance took to
    // land, the two drifted apart.
    utils::move_body(&mut app, planted, utils::point("8.5", "5.5"));
    utils::move_body(&mut app, ally, utils::point("7.5", "5.5"));
    utils::move_body(&mut app, shover, utils::point("6.9", "5.5"));

    utils::run_ticks(&mut app, 10);

    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, planted),
        utils::point("8.5", "5.5")
    );
    assert_eq!(utils::position_of(world, ally), utils::point("7.5", "5.5"));
    assert_eq!(
        utils::position_of(world, shover),
        utils::point("6.5", "5.5")
    );
}

/// In open ground a push slides around a body that will not give way rather
/// than stopping: an own ox set into a disabled ally pushes it toward an
/// idle enemy a little off the line of the push, and the ally slides along
/// the enemy's rim, ending off the straight line, touching the enemy.
#[test]
fn push_slides_body_around_enemy_in_open_ground() {
    let mut app = utils::continuous_orders_app();
    let (enemy, _) = utils::create_owned(&mut app, "ox", 8, 5, 1);
    let (ally, _) = utils::create_owned(&mut app, "ox", 7, 5, 0);
    let (shover, _) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::disable_for(&mut app, ally, 1000);
    utils::move_body(&mut app, enemy, utils::point("8.5", "5.7"));
    utils::move_body(&mut app, shover, utils::point("6.8", "5.5"));

    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    assert_eq!(utils::position_of(world, enemy), utils::point("8.5", "5.7"));
    // From (7.5, 5.5), a share of 0.15 of the 0.3 overlap aims it at
    // (7.65, 5.5), inside the enemy; it slides along the enemy's rim down and
    // away instead, to √(0.9733² + 0.2318²) ≈ 1.0005 from its middle.
    assert_eq!(
        utils::position_of(world, ally),
        utils::point("7.5266938973", "5.468206628")
    );
}

/// A body shut toward its push takes none of it: an own ox walking down a
/// one-cell corridor into a disabled ally that touches an idle enemy is
/// pushed back out of the ally whole each tick and ends each tick touching
/// it, where keeping its contest share alone it would settle three of its
/// steps deep.
#[test]
fn walker_never_presses_into_shut_body() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (enemy, _) = utils::create_owned(&mut app, "ox", 8, 5, 1);
    let (ally, _) = utils::create_owned(&mut app, "ox", 7, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::disable_for(&mut app, ally, 1000);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );

    let mut closest = FixedU64::MAX;
    for _ in 0..60 {
        utils::run_ticks(&mut app, 1);
        let world = app.world_mut();
        closest = closest
            .min(utils::position_of(world, walker).distance(utils::position_of(world, ally)));
    }

    // Never nearer than touching, 0.5 + 0.5 = 1, at the end of a tick.
    assert_eq!(closest, FixedU64::ONE);
    let world = app.world_mut();
    assert_eq!(utils::position_of(world, ally), utils::point("7.5", "5.5"));
    assert_eq!(utils::position_of(world, enemy), utils::point("8.5", "5.5"));
}

/// A body is no wall to a mover of a higher rank: two own bodies standing
/// their ground, set 0.4 into each other, halve the overlap, and the one
/// pushed toward an idle enemy touching it carries on into the enemy, which
/// takes that overlap whole the next tick — 5.3, 6.3 and 7.3 — where
/// stopping at the enemy's rim would leave the pair closing by halves.
#[test]
fn planted_body_pushes_into_idle_enemy() {
    let mut app = utils::continuous_orders_app();
    let (left, left_id) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (right, right_id) = utils::create_owned(&mut app, "soldier", 9, 9, 0);
    let (enemy, _) = utils::create_owned(&mut app, "ox", 12, 12, 1);
    for id in [left_id, right_id] {
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::SetStance {
                stance: Stance::StandGround,
            },
        );
    }
    utils::run_ticks(&mut app, 5);
    utils::move_body(&mut app, right, utils::point("6.1", "5.5"));
    utils::move_body(&mut app, enemy, utils::point("7.1", "5.5"));

    utils::run_ticks(&mut app, 2);

    let world = app.world_mut();
    assert_eq!(utils::position_of(world, left), utils::point("5.3", "5.5"));
    assert_eq!(utils::position_of(world, right), utils::point("6.3", "5.5"));
    assert_eq!(utils::position_of(world, enemy), utils::point("7.3", "5.5"));
}

/// Only idle units make way for a walker: an own soldier standing at its
/// attack on an enemy rampart beside it is ground to a walker passing
/// through its cell, which slides around it, and the soldier keeps its place.
#[test]
fn walker_never_shoves_unit_at_work() {
    let mut app = utils::continuous_orders_app();
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    let (target, target_id) = utils::create_owned(&mut app, "rampart", 5, 4, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 30);

    let world = app.world_mut();
    assert!(!utils::order_queue_is_empty(world, soldier));
    assert!(world.get_entity(target).is_ok());
    assert_eq!(
        utils::position_of(world, soldier),
        utils::point("5.5", "5.5")
    );
    // The walker slid around it and reached its spot, (9.5, 5.5).
    assert!(utils::order_queue_is_empty(world, walker));
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("9.5", "5.5")
    );
}

/// A walker disabled within a tick, after its walk stepped, is inert to that
/// tick's contact: an own obelisk acting after it in the tick stuns it as it
/// presses an idle ally, and the pair parts as an inert body and an idle one
/// do, where a walker pressing on would hold its line three times as hard.
#[test]
fn walker_disabled_mid_tick_is_inert_to_contact() {
    let mut app = utils::continuous_orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let stunned = registry.register_entity_buff(
            "stunned",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::For(100),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
        let stun = registry.register_skill(
            "stun",
            SkillDef {
                cooldown: 100,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::ApplyBuff {
                        buff: stunned,
                        on: Recipient::Standing {
                            whom: Whom {
                                relations: Relation::Own.into(),
                                kinds: Kinds::Any,
                            },
                            reach: Reach::Within(Area::Around(Quantity::from(8))),
                        },
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("obelisk")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    None,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(100))
                .with_skills([stun]),
        );
    }
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    let (ally, _) = utils::create_owned(&mut app, "soldier", 4, 5, 0);
    let (_, obelisk_id) = utils::create_owned(&mut app, "obelisk", 4, 8, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::cast_on(&mut app, obelisk_id, "stun", walker_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let world = app.world_mut();
    // The rest of the pair, pinned to the bit: read as a walker pressing on
    // the tick it is stunned, it would stand at (5.0845, 5.6463), holding its
    // line against the ally three times as hard.
    assert_eq!(
        utils::position_of(world, walker),
        utils::position_bits(0x5_0501_1b01, 0x5_b909_9c73)
    );
    assert_eq!(
        utils::position_of(world, ally),
        utils::position_bits(0x5_aab6_b0ad, 0x4_f5e7_de36)
    );
}

/// A walk's heading on its last leg is the ordered spot itself, the aim its
/// step takes: a soldier standing on its goal cell's middle, 0.45 short of
/// a spot east of it, heads east and presses an idle ally it overlaps to its
/// north-east, holding its line three times as hard — where a heading to the
/// cell's middle, where it stands, would press nothing.
#[test]
fn last_leg_heads_where_its_step_aims() {
    let mut app = utils::continuous_orders_app();
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 8, 10, 0);
    let (ally, _) = utils::create_owned(&mut app, "soldier", 10, 9, 0);
    utils::move_body(&mut app, ally, utils::point("10.6", "9.52"));
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::point("10.95", "10.5"),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 3);

    // Four steps of 0.5 along row 10.5 from (8.5, 10.5) end on (10.5, 10.5),
    // 0.985 from the ally — overlapping it by 0.015 — which the walker,
    // pressing, gives up only a quarter of; heading nowhere it would give up
    // half: (10.4992, 10.5074) and the ally (10.5989, 9.5124).
    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, walker),
        utils::position_bits(0xa_7fe7_3321, 0xa_80f3_0ae7)
    );
    assert_eq!(
        utils::position_of(world, ally),
        utils::position_bits(0xa_99e4_0034, 0x9_8245_979b)
    );
}

/// A click the walk cannot reach ends on meeting a body holding the cell the
/// walk aims at instead: ordered into a walled block, the walk heads for the
/// reachable cell its plan ends on, (9, 3), where an idle enemy is placed
/// once the walk is planned, and ends on meeting it rather than pressing on
/// toward the click.
#[test]
fn walk_to_unreachable_click_ends_on_meeting_holder_of_its_end() {
    let mut app = utils::continuous_orders_app();
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 10..13 {
            for y in 3..8 {
                map.set_static_occupied(utils::GROUND, CellPos::new(x, y), true);
            }
        }
    }
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 3, 5, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::create_owned(&mut app, "ox", 9, 3, 1);

    let ended = (1..200).find(|_| {
        utils::run_ticks(&mut app, 1);
        utils::order_queue_is_empty(app.world_mut(), walker)
    });

    // On the eighth tick at ≈ (8.5139, 3.7041), ≈ 1.007 from the enemy's
    // middle (9.5, 3.5): within the meeting slack of touching it, 0.5 + 0.5
    // = 1 apart — its last step aimed all but dead at that middle, the side
    // it slides to decided by the last bits; measured against the click it would press on until its
    // acceptance ring ends it, on the eightieth.
    assert_eq!(ended, Some(8));
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::position_bits(0x8_8390_aeac, 0x3_b43d_1363)
    );
}

/// A long walk aims somewhere on every tick it has crossings ahead: the tick
/// a segment's last waypoint is reached, the next segment is refined at
/// once, so the walk never stands between segments with no waypoint — across
/// the choke-point map's wall it would on nine ticks.
#[test]
fn long_walk_always_has_waypoint_ahead() {
    let mut app = utils::continuous_orders_app();
    utils::install_choke_point_map(&mut app, MovementModel::Continuous);
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 10, 10, 0);

    let empty = empty_ticks_walking_to_far_corner(&mut app, walker, walker_id);

    assert_eq!(empty, 0);
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::point("85.5", "80.5")
    );
}

/// A segment's last waypoint dropped at a holder that will not give way
/// takes the next segment at once, as one reached does: a soldier standing
/// its ground on (32, 15), the second segment's end, is met and the waypoint
/// dropped, and the walk still aims somewhere on every tick.
#[test]
fn dropped_segment_end_takes_next_segment_at_once() {
    let mut app = utils::continuous_orders_app();
    utils::install_choke_point_map(&mut app, MovementModel::Continuous);
    let (_, holder_id) = utils::create_owned(&mut app, "soldier", 32, 15, 0);
    utils::select(&mut app, holder_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    let (walker, walker_id) = utils::create_owned(&mut app, "soldier", 10, 10, 0);

    let empty = empty_ticks_walking_to_far_corner(&mut app, walker, walker_id);

    assert_eq!(empty, 0);
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::point("85.5", "80.5")
    );
}

/// An idle ally on a waypoint does not make the walker drop it: it gives way
/// to a walker, so the walker that meets the idle soldier standing on the
/// waypoint (6, 6) keeps pursuing it, where a soldier at work would have
/// made it drop the waypoint.
#[test]
fn walker_keeps_waypoint_held_by_idle_ally() {
    let mut app = utils::continuous_orders_app();
    walled_route(&mut app);
    utils::create_owned(&mut app, "soldier", 6, 6, 0);
    utils::create_owned(&mut app, "rampart", 7, 7, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 2, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 8),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The first leg, (2.5, 2.5) to the waypoint (4, 6), ends on the seventh
    // tick at (4.5, 6.5); then half a cell a tick along row 6.5, the walker
    // meets the soldier on the ninth at x 5.5, 1 short of its middle.
    utils::run_ticks(&mut app, 9);
    let world = app.world_mut();
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("5.5", "6.5")
    );
    assert_eq!(
        world
            .get::<MoveComponent>(walker)
            .map(|walk| walk.path.clone()),
        Some(vec![
            CellPos::new(12, 8),
            CellPos::new(10, 2),
            CellPos::new(8, 2),
            CellPos::new(6, 6)
        ])
    );
}

/// A tick that drops a held waypoint tests no arrival against it: an own
/// soldier at its attack stands 0.5 past the waypoint (6, 6) on the
/// walker's way, so the walker meets it 0.5 short of that waypoint — within
/// the slack an intermediate waypoint pops in — and drops it, still
/// pursuing the next one, (8, 2), rather than dropping that too.
#[test]
fn dropped_waypoint_takes_no_other_with_it() {
    let mut app = utils::continuous_orders_app();
    walled_route(&mut app);
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 6, 6, 0);
    utils::move_body(&mut app, soldier, utils::point("7", "6.5"));
    let (_, rampart_id) = utils::create_owned(&mut app, "rampart", 7, 7, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 2, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(rampart_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 8),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // Waypoints (4, 6), (6, 6), (8, 2), (10, 2), (12, 8): the first leg ends
    // on the seventh tick at (4.5, 6.5); then half a cell a tick along row
    // 6.5, it stands at x 6 on the tenth, touching the soldier, 7 − 6 = 1
    // apart, and 0.5 short of (6.5, 6.5).
    utils::run_ticks(&mut app, 10);
    let world = app.world_mut();
    assert_eq!(utils::position_of(world, walker), utils::point("6", "6.5"));
    assert_eq!(
        world
            .get::<MoveComponent>(walker)
            .map(|walk| walk.path.clone()),
        Some(vec![
            CellPos::new(12, 8),
            CellPos::new(10, 2),
            CellPos::new(8, 2)
        ])
    );
}

/// A waypoint a body that is not walking stands on is reached on meeting
/// it: a walker routed through a gap in a wall to the waypoint (6, 6), where
/// an own soldier stands at its attack, drops that waypoint on meeting the
/// soldier and walks on around it to its goal, arriving on the pinned tick.
#[test]
fn walker_passes_waypoint_held_by_unit_at_work() {
    let mut app = utils::continuous_orders_app();
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for y in 0..13 {
            if !(6..=8).contains(&y) {
                map.set_static_occupied(utils::GROUND, CellPos::new(5, y), true);
            }
        }
    }
    let (_, soldier_id) = utils::create_owned(&mut app, "soldier", 6, 6, 0);
    let (_, rampart_id) = utils::create_owned(&mut app, "rampart", 6, 7, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 2, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(rampart_id),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 2),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    let mut arrived = None;
    for tick in 1..120 {
        utils::run_ticks(&mut app, 1);
        if utils::order_queue_is_empty(app.world_mut(), walker) {
            arrived = Some(tick);
            break;
        }
    }

    // Thirty-three ticks after its command lands; held to that waypoint it
    // would wait out its stall clock against the soldier and arrive on the
    // seventy-first.
    assert_eq!(arrived, Some(33));
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::point("9.5", "2.5")
    );
}

/// A walk whose way a body that will not give way blocks for good gives up:
/// in a one-cell corridor behind an own ox standing its ground, the walker's
/// detours run through the ox's cell, and each held waypoint it drops is no
/// progress, so the escalations climb to the give-up budget and the walk
/// ends on the pinned tick.
#[test]
fn walk_blocked_for_good_by_standing_body_gives_up() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (_, planted_id) = utils::create_owned(&mut app, "ox", 8, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::select(&mut app, planted_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SetStance {
            stance: Stance::StandGround,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    let mut ended = None;
    for tick in 1..1000 {
        utils::run_ticks(&mut app, 1);
        if utils::order_queue_is_empty(app.world_mut(), walker) {
            ended = Some(tick);
            break;
        }
    }

    // On the fifty-sixth tick after its command lands; counting each dropped
    // waypoint as reached, the walk would wait behind the ox for good.
    assert_eq!(ended, Some(56));
    assert_eq!(
        utils::position_of(app.world_mut(), walker),
        utils::point("7.5", "5.5")
    );
}

/// A walk whose spot an idle ally stands on ends on meeting it, touching:
/// sent onto (8.5, 5.5), where an own idle ox stands, a walker from the west
/// stops 1 short, at (7.5, 5.5), without shoving the ally off; the two idle
/// oxen then drift 0.125 each to the idle spacing, 1.25 apart.
#[test]
fn walk_onto_idle_ally_spot_stops_touching_it() {
    let mut app = utils::continuous_orders_app();
    let (ally, _) = utils::create_owned(&mut app, "ox", 8, 5, 0);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 60);

    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, walker));
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("7.375", "5.5")
    );
    assert_eq!(
        utils::position_of(world, ally),
        utils::point("8.625", "5.5")
    );
}

/// A walk whose spot an idle enemy stands on ends at the enemy's rim: the
/// enemy does not give way, so the walker stops where it first touches it,
/// (7.5, 5.5), and the walk ends there.
#[test]
fn walk_onto_idle_enemy_spot_stops_at_its_rim() {
    let mut app = utils::continuous_orders_app();
    let (enemy, _) = utils::create_owned(&mut app, "ox", 8, 5, 1);
    let (walker, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 5),
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 60);

    // At the enemy's rim, touching it: 8.5 − (0.5 + 0.5) = 7.5.
    let world = app.world_mut();
    assert!(utils::order_queue_is_empty(world, walker));
    assert_eq!(
        utils::position_of(world, walker),
        utils::point("7.5", "5.5")
    );
    assert_eq!(utils::position_of(world, enemy), utils::point("8.5", "5.5"));
}

/// Idle enemies keep no spacing: touching, both keep the middles they were
/// placed on, 1 apart, where an idle pair of one side drifts to 1.25.
#[test]
fn idle_enemies_touching_keep_their_places() {
    let mut app = utils::continuous_orders_app();
    let (own, _) = utils::create_owned(&mut app, "ox", 5, 5, 0);
    let (enemy, _) = utils::create_owned(&mut app, "ox", 6, 5, 1);

    utils::run_ticks(&mut app, 50);

    let world = app.world_mut();
    assert_eq!(utils::position_of(world, own), utils::point("5.5", "5.5"));
    assert_eq!(utils::position_of(world, enemy), utils::point("6.5", "5.5"));
}

//
// ─── Nobody's bodies ──────────────────────────────────────────────────────────
//

/// A body that is nobody's meets an owned one as an ally does: touching and
/// idle, the two drift 0.125 each to the 0.25 spacing, 1.25 apart.
#[test]
fn idle_unowned_body_keeps_spacing_with_owned_one() {
    let mut app = utils::continuous_orders_app();
    let (own, _) = utils::create_owned(&mut app, "ox", 5, 5, 0);
    let (unowned, _) = utils::create_entity(app.world_mut(), "ox", utils::cell(6, 5), None)
        .expect("the cell beside is free");

    utils::run_ticks(&mut app, 50);

    let world = app.world_mut();
    assert_eq!(utils::position_of(world, own), utils::point("5.375", "5.5"));
    assert_eq!(
        utils::position_of(world, unowned),
        utils::point("6.625", "5.5")
    );
}

/// A walker shoves an idle body that is nobody's down a corridor its own
/// width, as it shoves an idle ally.
#[test]
fn walker_shoves_idle_unowned_body() {
    let mut app = utils::continuous_orders_app();
    walled_row(&mut app);
    let (unowned, _) = utils::create_entity(app.world_mut(), "ox", utils::cell(6, 5), None)
        .expect("the corridor cell is free");
    let (_, walker_id) = utils::create_owned(&mut app, "ox", 2, 5, 0);

    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    let world = app.world_mut();
    // Plowed down the corridor until the walker stands as near its spot
    // (11.5, 5.5) as the body lets it, where the walk ends.
    assert_eq!(
        utils::position_of(world, unowned),
        utils::point("11.375", "5.5")
    );
}

//
// ─── Body radius ──────────────────────────────────────────────────────────────
//

#[test]
fn body_spans_its_circles_square() {
    // A 1×1 footprint with radius 1 is a body max(1, ⌈2 × 1⌉) = 2 cells a
    // side: created at (5, 5) it claims (5..=6, 5..=6), its circle around the
    // square's middle (5 + 2/2, 5 + 2/2) = (6, 6).
    let mut app = bodied_app(CellSize::ONE, "1");
    let (body, _) = utils::create_owned(&mut app, "bodied", 5, 5, 0);
    let world = app.world();

    assert_eq!(entity_def::extent(world, body), CellSize::new(2, 2));
    assert_eq!(
        entity_def::claim_on(world, body, utils::GROUND.into()),
        Some(CellRect::new(CellPos::new(5, 5), CellSize::new(2, 2)))
    );
    assert_eq!(
        entity_def::shape(world, body),
        Shape::Circle {
            middle: utils::point("6", "6"),
            radius: FixedU64::ONE,
        }
    );
}

#[test]
fn wide_circle_is_refused_one_cell_gap() {
    // A 1×1 footprint with radius 1 plans as its 2×2 square, which no
    // anchor fits through the one open cell (10, 10) of the wall down column
    // 10: sent from (5, 9) to (15, 9) it stays west of the wall. Its goal is
    // out of reach, so the walk ends at the nearest anchor it reaches. At
    // anchor (8, y) its body spans columns 8..=9 and rows y..=y + 1: the x gap
    // to the goal is 15 − 9 = 6, and the y gap is at most 6 for every y in
    // 2..=15 (y = 2: 9 − (2 + 1) = 6), so every such anchor lies 6 away and
    // the tie breaks on cell order to (8, 2).
    let mut app = bodied_app(CellSize::ONE, "1");
    wall_with_gap(&mut app);
    let (body, _) = utils::create_owned(&mut app, "bodied", 5, 9, 0);
    utils::order_move(&mut app, body, 15, 9);
    utils::run_ticks(&mut app, 80);

    assert_eq!(utils::anchor_of(app.world(), body), CellPos::new(8, 2));
}

#[test]
fn narrow_circle_on_wide_footprint_passes_one_cell_gap() {
    // A 3×3 footprint with radius 0.5 is a body max(1, ⌈2 × 0.5⌉) = 1 cell
    // a side, which walks through the one open cell (10, 10) of the wall down
    // column 10, from (5, 10) to (15, 10).
    let mut app = bodied_app(CellSize::new(3, 3), "0.5");
    wall_with_gap(&mut app);
    let (body, _) = utils::create_owned(&mut app, "bodied", 5, 10, 0);
    utils::order_move(&mut app, body, 15, 10);
    utils::run_ticks(&mut app, 80);

    assert_eq!(utils::anchor_of(app.world(), body), CellPos::new(15, 10));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Walls rows 4 and 6 from column 1 to 12, leaving row 5 a corridor one cell
/// wide.
fn walled_row(app: &mut App) {
    let world = app.world_mut();
    let mut map = world.resource_mut::<Map>();
    for x in 1..13 {
        map.set_static_occupied(utils::GROUND, CellPos::new(x, 4), true);
        map.set_static_occupied(utils::GROUND, CellPos::new(x, 6), true);
    }
}

/// The orders roster on a continuous-model map, with a `slim_wagon`: a mover
/// whose circle of radius 0.9 sits inside its 2×2 body.
fn slim_wagon_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    app.world_mut().resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("slim_wagon")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::new(2, 2),
                Some(utils::fixed("0.9")),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_movement(
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
            .with_pool(Pool::health(60))
            .with_dying(2, []),
    );
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    app
}

/// A continuous app whose row 10 is a corridor from x 2 to 9, walled at
/// (10, 10), with oxen of `owner` idle at (7, 10), (8, 10) and (9, 10), and a
/// soldier of player 0 at (3, 10) ordered to the middle of (9, 10).
fn oxen_in_corridor(owner: u8) -> (App, Entity, Vec<Entity>) {
    let mut app = utils::continuous_orders_app();
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 2..=11 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 9), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 11), true);
        }
        map.set_static_occupied(utils::GROUND, CellPos::new(10, 10), true);
    }
    let oxen = (7..=9)
        .map(|x| utils::create_owned(&mut app, "ox", x, 10, owner).0)
        .collect();
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 3, 10, 0);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(9, 10),
            flush: true,
        },
    );
    (app, soldier, oxen)
}

/// Two soldiers walking in a column, stalled behind an enemy ox.
struct Column {
    /// The front soldier.
    front: Entity,
    /// The rear soldier.
    rear: Entity,
    /// The enemy ox and its id.
    blocker: (Entity, SimulationId),
    /// The id credited with the blocker's death.
    killer: SimulationId,
}

/// Which soldier of a column is created first.
#[derive(Debug, Clone, Copy)]
enum Created {
    /// The front one.
    FrontFirst,
    /// The rear one.
    RearFirst,
}

/// Two own soldiers in a corridor one cell high, the front one at (5, 10)
/// sent to (12, 10) and the rear one at (4, 10) to (11, 10), stalled by an
/// idle enemy ox at (8, 10), created in the `created` order.
fn column_behind_enemy(created: Created) -> (App, Column) {
    let mut app = utils::continuous_orders_app();
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for x in 2..=14 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 9), true);
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 11), true);
        }
    }
    let blocker = utils::create_owned(&mut app, "ox", 8, 10, 1);
    let mut soldiers = [(5, 12), (4, 11)].map(|(x, to)| (x, to, None::<(Entity, SimulationId)>));
    let order: [usize; 2] = match created {
        Created::FrontFirst => [0, 1],
        Created::RearFirst => [1, 0],
    };
    for index in order {
        let (x, _, ref mut made) = soldiers[index];
        *made = Some(utils::create_owned(&mut app, "soldier", x, 10, 0));
    }
    for (_, to, made) in soldiers {
        let (_, id) = made.expect("each soldier is made");
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(to, 10),
                flush: true,
            },
        );
    }
    let [(_, _, front), (_, _, rear)] = soldiers;
    let (front, killer) = front.expect("the front soldier is made");
    let (rear, _) = rear.expect("the rear soldier is made");
    (
        app,
        Column {
            front,
            rear,
            blocker,
            killer,
        },
    )
}

/// The front and rear soldiers' frustration.
fn frustrations(app: &App, column: &Column) -> (u32, u32) {
    let of = |entity: Entity| {
        app.world()
            .get::<MoveComponent>(entity)
            .expect("the soldier still walks")
            .frustration
    };
    (of(column.front), of(column.rear))
}

/// The orders roster on a continuous-model map, with a `bodied` mover whose
/// footprint is `footprint` and whose circle has radius `radius`.
fn bodied_app(footprint: CellSize, radius: &str) -> App {
    paced_app(footprint, radius, "0.5")
}

/// The orders roster on a continuous-model map, with a `bodied` mover whose
/// footprint is `footprint`, whose circle has radius `radius` and whose speed
/// is `speed`.
fn paced_app(footprint: CellSize, radius: &str, speed: &str) -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    app.world_mut().resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("bodied")
            .with_location(
                utils::GROUND_DOMAIN,
                footprint,
                Some(utils::fixed(radius)),
                Solidity::Solid,
                utils::GROUND,
            )
            .with_movement(
                utils::fixed(speed),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    app
}

/// Walls column 10 on the ground but for the one cell (10, 10).
fn wall_with_gap(app: &mut App) {
    let mut map = app.world_mut().resource_mut::<Map>();
    for y in (0..32).filter(|&y| y != 10) {
        map.set_static_occupied(utils::GROUND, CellPos::new(10, y), true);
    }
}

/// Walls column 5 from row 0 to 12 but for a gap at rows 6–8, and column 9
/// from row 3 to 12: a route from (2, 2) to (12, 8) through (4, 6), (6, 6),
/// (8, 2) and (10, 2).
fn walled_route(app: &mut App) {
    let mut map = app.world_mut().resource_mut::<Map>();
    for y in 0..13 {
        if !(6..=8).contains(&y) {
            map.set_static_occupied(utils::GROUND, CellPos::new(5, y), true);
        }
        if y >= 3 {
            map.set_static_occupied(utils::GROUND, CellPos::new(9, y), true);
        }
    }
}

/// Orders `walker` across the choke-point map to (85, 80) and counts the
/// ticks of the next 400 it has crossings ahead and no waypoint.
fn empty_ticks_walking_to_far_corner(
    app: &mut App,
    walker: Entity,
    walker_id: SimulationId,
) -> u32 {
    utils::select(app, walker_id);
    utils::push_command(
        app,
        PlayerCommand::Move {
            target: utils::pos(85, 80),
            flush: true,
        },
    );
    utils::run_ticks(app, utils::APPLY + 1);
    let mut empty = 0;
    for _ in 0..400 {
        utils::run_ticks(app, 1);
        match app.world().get::<MoveComponent>(walker) {
            Some(walk) if walk.path.is_empty() && !walk.corridor.is_empty() => empty += 1,
            Some(_) | None => {}
        }
    }
    empty
}
