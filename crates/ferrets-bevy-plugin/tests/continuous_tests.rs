//! The continuous movement model: free positions, bodies pushing apart, and
//! terrain-checked commits.
//!
//! Rest positions and cells are asserted exactly where they are pinned:
//! every value is deterministic fixed-point math replayed identically on
//! every lockstep peer, so a drifted equilibrium — however plausible — is
//! a desync. Changing a pinned value must be a conscious decision.

mod utils;

use bevy::prelude::App;
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::Solidity, pool::Pool,
    registry::ContentRegistry, stats::ModifierOp,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    entity_def,
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
    session::{player_slot::PlayerSlot, player_type::PlayerType},
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
    // half a body each way, so they rest a full diameter apart, each
    // holding its own cell.
    assert_eq!(a, utils::point("5", "5.5"));
    assert_eq!(b, utils::point("6", "5.5"));
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
            CellPos::new(7, 5),
            CellPos::new(7, 4),
            CellPos::new(7, 6),
            CellPos::new(7, 2),
            CellPos::new(7, 3),
        ]
    );
    // Each rests flush against the water: its middle at x = 7.5 reaches to
    // 7.5 + 0.5 = 8, the water's edge.
    let edges: Vec<FixedU64> = landlubbers
        .iter()
        .map(|(landlubber, _)| utils::position_of(world, *landlubber).x)
        .collect();
    assert_eq!(edges, vec![utils::fixed("7.5"); 5]);
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

    // The exact settle against the wall, in spawn order: a column pressed
    // one cell short of it around the taken goal, the straggler behind the
    // pile.
    {
        let world = app.world_mut();
        let cells: Vec<CellPos> = soldiers
            .iter()
            .map(|(soldier, _)| utils::anchor_of(world, *soldier))
            .collect();
        assert_eq!(
            cells,
            vec![
                CellPos::new(7, 5),
                CellPos::new(7, 4),
                CellPos::new(7, 6),
                CellPos::new(7, 2),
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

    // The exact settle: a compact block around the contested point, one
    // body per cell.
    let mut cells: Vec<CellPos> = soldiers
        .iter()
        .map(|(soldier, _)| utils::anchor_of(app.world(), *soldier))
        .collect();
    cells.sort_unstable();
    assert_eq!(
        cells,
        vec![
            CellPos::new(5, 4),
            CellPos::new(5, 5),
            CellPos::new(6, 4),
            CellPos::new(6, 5),
            CellPos::new(7, 5),
            CellPos::new(7, 6),
        ]
    );
}

#[test]
fn crowd_of_ten_rests_one_per_cell() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A packed double column marching onto one point — circles pack tighter
    // than one per cell, so without the rest-separation nudge some pair
    // ends up sharing a cell and occupancy stops being countable.
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

    utils::run_ticks(&mut app, 800);

    let world = app.world_mut();
    for (soldier, _) in &soldiers {
        assert!(
            utils::order_queue_is_empty(world, *soldier),
            "every walk into the crowd must finish"
        );
    }
    let mut cells: Vec<CellPos> = soldiers
        .iter()
        .map(|(soldier, _)| utils::anchor_of(world, *soldier))
        .collect();
    cells.sort_unstable();
    cells.dedup();
    // The exact settle: a compact block around the contested point.
    assert_eq!(
        cells,
        vec![
            CellPos::new(11, 3),
            CellPos::new(11, 5),
            CellPos::new(12, 2),
            CellPos::new(12, 3),
            CellPos::new(12, 4),
            CellPos::new(12, 5),
            CellPos::new(13, 2),
            CellPos::new(13, 4),
            CellPos::new(13, 5),
            CellPos::new(14, 3),
        ]
    );
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
    // The exact end state: the walker ring-accepts beside its contested
    // goal, the idle body plowed one cell past it.
    assert_eq!(utils::anchor_of(world, idle), CellPos::new(12, 5));
    assert_eq!(utils::anchor_of(world, walker), CellPos::new(11, 5));
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
    // A slim wagon — 2×2, its circle of radius 0.4 well inside its footprint
    // — rests against the east edge of the 32-cell map, anchored at (30, 5),
    // its middle at (31, 6). A soldier set inside its north-west quarter
    // pushes it south-east, where its circle would still sit on the map but
    // its body would reach past the edge: the push's east share is held at
    // x 32 − 2/2 = 31, and its south share slides the wagon along the edge
    // into row 6, its footprint still ending on column 31.
    let mut app = slim_wagon_app();
    let (wagon, _) = utils::create_owned(&mut app, "slim_wagon", 30, 5, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 9, 9, 0);
    utils::move_body(&mut app, soldier, utils::pos(30, 5));

    utils::run_ticks(&mut app, 300);

    assert_eq!(
        utils::anchor_of(app.world_mut(), wagon),
        CellPos::new(30, 6)
    );
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
    // A slim wagon — 2×2, its circle of radius 0.4 — fits by its circle with
    // its middle 0.4 from an edge, but its body reaches a whole cell either
    // side: ordered to x 0.5 it finishes at 2/2 = 1, and to x 31.8 on the
    // 32-wide map at 32 − 1 = 31.
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
    // middles, each carried half the overlap per pass until contact clears.
    assert_eq!(
        utils::position_of(world, wagon),
        utils::position_bits(0x5_b83c_499a, 0x5_b83c_499a)
    );
    assert_eq!(
        utils::position_of(world, soldier),
        utils::position_bits(0x6_c7c3_b666, 0x6_c7c3_b666)
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

    // The walk presses on the ox at (9,10) once it reaches (8,10), crumbs of
    // progress holding its stall clock off until the thirty-ninth; fifteen ticks
    // on, on the fifty-fourth, the escalation routes it round the walls to
    // (10,9) above the rampart, where its first blow of 10 lands on the
    // seventy-second.
    utils::run_ticks(&mut app, 71);
    assert_eq!(utils::health_as_u32(&app, rampart), 400);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, rampart), 390);
    assert_eq!(
        utils::position_of(app.world_mut(), soldier),
        utils::point("10.5", "9")
    );
}

//
// ─── Body radius ──────────────────────────────────────────────────────────────
//

#[test]
fn buffed_radius_is_held_to_half_its_footprint() {
    // The 1×1 soldier's half-cell radius buffed by a whole cell: 0.5 + 1 =
    // 1.5 read as 0.5, half its footprint's narrow side, so its body still
    // lies over (5, 5) alone.
    let mut app = utils::continuous_orders_app();
    let swollen = utils::register_entity_buff(
        &mut app,
        "swollen",
        EntityStatId::RADIUS,
        ModifierOp::FlatAdd,
        "1",
        None,
    );
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    utils::apply_buff(app.world_mut(), soldier, swollen);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        entity_def::effective_stat(app.world(), soldier, EntityStatId::RADIUS),
        Some(utils::fixed("1.5"))
    );
    assert_eq!(
        entity_def::radius(app.world(), soldier),
        utils::fixed("0.5")
    );
    assert_eq!(
        entity_def::covered_rect(app.world(), soldier),
        CellRect::cell(CellPos::new(5, 5))
    );
}

#[test]
fn body_shrunk_to_no_radius_covers_cell_its_middle_lies_in() {
    // The soldier's half-cell radius taken away whole: 0.5 − 0.5 = 0. Its
    // middle walked onto (6, 6), the corner four cells share, lies in (6, 6),
    // the one cell its body covers.
    let mut app = utils::continuous_orders_app();
    let shrunk = utils::register_entity_buff(
        &mut app,
        "shrunk",
        EntityStatId::RADIUS,
        ModifierOp::FlatAdd,
        "-0.5",
        None,
    );
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 5, 5, 0);
    utils::walk_onto(&mut app, soldier, utils::point("6", "6"), 10);
    utils::apply_buff(app.world_mut(), soldier, shrunk);
    utils::run_ticks(&mut app, 1);

    assert_eq!(entity_def::radius(app.world(), soldier), FixedU64::ZERO);
    assert_eq!(
        entity_def::covered_rect(app.world(), soldier),
        CellRect::cell(CellPos::new(6, 6))
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The orders roster on a continuous-model map, with a `slim_wagon`: a 2×2
/// mover whose circle has radius 0.4.
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
                Solidity::Solid,
                utils::GROUND,
            )
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.4"),
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
