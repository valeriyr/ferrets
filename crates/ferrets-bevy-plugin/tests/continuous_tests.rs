//! The continuous movement model: free positions, bodies pushing apart, and
//! terrain-checked commits.
//!
//! Rest positions and cells are asserted exactly where they are pinned:
//! every value is deterministic fixed-point math replayed identically on
//! every lockstep peer, so a drifted equilibrium — however plausible — is
//! a desync. Changing a pinned value must be a conscious decision.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_physics::body;
use ferrets_simulation::{
    command::PlayerCommand, components::location::LocationComponent, map::Map,
    movement_model::MovementModel,
};

#[test]
fn continuous_walk_reaches_goal() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (soldier, id) = utils::spawn_owned(&mut app, "soldier", 2, 2, 0);

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
        utils::cell_of(app.world_mut(), soldier),
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
    let first = utils::spawn_owned(&mut app, "soldier", 3, 5, 0);
    let second = utils::spawn_owned(&mut app, "soldier", 7, 5, 0);
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
    // Bodies are circles on every projection, so rest separation is
    // Euclidean regardless of the map's own metric.
    let separation = a.distance(b);
    assert!(
        separation >= FixedU64::from_num(0.99),
        "bodies must rest a full body apart: separation {separation}"
    );
    // The exact equilibrium: both walks touch the contested spot the same
    // tick and finish there; the pushing pass parts the coincident pair
    // half a body each way, so they rest a full diameter apart, each
    // holding its own cell.
    assert_eq!(a, utils::position_bits(0x4_8000_0000, 0x5_0000_0000));
    assert_eq!(b, utils::position_bits(0x5_8000_0000, 0x5_0000_0000));
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
        .map(|&(x, y)| utils::spawn_owned(&mut app, "soldier", x, y, 0))
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
            .map(|(soldier, _)| body::anchor(utils::position_of(world, *soldier)))
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
        let half = FixedU64::from_num(0.5);
        let center = (position.x + half, position.y + half);
        let radius = half;
        let cells_of = |value: FixedU64| {
            let first = value.floor().to_num::<u32>();
            if value.frac() == FixedU64::ZERO {
                first..=first
            } else {
                first..=first + 1
            }
        };
        for y in cells_of(position.y) {
            for x in cells_of(position.x) {
                let nearest = |value: FixedU64, cell: u32| {
                    value.clamp(FixedU64::from_num(cell), FixedU64::from_num(cell + 1))
                };
                let off_x = center.0.abs_diff(nearest(center.0, x)).to_bits() as u128;
                let off_y = center.1.abs_diff(nearest(center.1, y)).to_bits() as u128;
                let reach = radius.to_bits() as u128;
                if off_x * off_x + off_y * off_y >= reach * reach {
                    continue;
                }
                assert!(
                    world
                        .resource::<Map>()
                        .nav_grid()
                        .is_statically_passable_by(utils::GROUND, CellPos::new(x, y)),
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
    let (left, left_id) = utils::spawn_owned(&mut app, "soldier", 2, 5, 0);
    let (right, right_id) = utils::spawn_owned(&mut app, "soldier", 9, 5, 0);

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

    assert_eq!(utils::cell_of(app.world_mut(), left), CellPos::new(9, 5));
    assert_eq!(utils::cell_of(app.world_mut(), right), CellPos::new(2, 5));
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
        .map(|&(x, y)| utils::spawn_owned(&mut app, "soldier", x, y, 0))
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

    // And rest means separated bodies: no pair closer than a body diameter.
    for (index, a) in settled.iter().enumerate() {
        for b in settled.iter().skip(index + 1) {
            let separation = a.distance(*b);
            assert!(
                separation >= FixedU64::from_num(0.99),
                "settled bodies must not overlap: separation {separation}"
            );
        }
    }
    // The exact settle: a compact block around the contested point, one
    // body per cell.
    let mut cells: Vec<CellPos> = settled.iter().map(|&body| body::anchor(body)).collect();
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
        .map(|i| utils::spawn_owned(&mut app, "soldier", 2 + i % 2, 2 + i / 2, 0))
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
        .map(|(soldier, _)| {
            body::anchor(
                world
                    .entity(*soldier)
                    .get::<LocationComponent>()
                    .unwrap()
                    .position,
            )
        })
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
    let (idle, _) = utils::spawn_owned(&mut app, "soldier", 6, 5, 0);
    let (walker, walker_id) = utils::spawn_owned(&mut app, "soldier", 2, 5, 0);

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
    let cell = utils::cell_of(world, idle);
    assert!(
        cell.x > 6 && cell.y == 5,
        "idle body must be plowed down the corridor: rests on {cell:?}"
    );
    // The exact end state: the walker ring-accepts beside its contested
    // goal, the idle body plowed one cell past it.
    assert_eq!(
        body::anchor(utils::position_of(world, idle)),
        CellPos::new(12, 5)
    );
    assert_eq!(
        body::anchor(utils::position_of(world, walker)),
        CellPos::new(11, 5)
    );
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_claimed_by(utils::GROUND, CellPos::new(6, 5)),
        "pushed body must release the cell it was shoved off"
    );

    // The claim plane holds one cell per body — the cell under its center,
    // the one it visually stands on: every corridor cell is claimed exactly
    // when it is some body's center cell, however the bodies straddle
    // borders.
    let bodies: Vec<_> = [idle, walker]
        .iter()
        .map(|&entity| utils::position_of(world, entity))
        .collect();
    for x in 1..13 {
        let cell = CellPos::new(x, 5);
        let expected = bodies.iter().any(|&body| body::anchor(body) == cell);
        assert_eq!(
            world
                .resource::<Map>()
                .nav_grid()
                .is_claimed_by(utils::GROUND, cell),
            expected,
            "claim on {cell:?} must be the cell under a body's center"
        );
    }
}

#[test]
fn wide_body_claims_whole_footprint() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A resting 2x2 body claims all four cells of its footprint — the claim
    // plane is rebuilt as wide as the body moves, not one cell under its
    // anchor.
    let _ = utils::spawn_owned(&mut app, "wagon", 5, 5, 0);
    utils::run_ticks(&mut app, 2);

    let world = app.world_mut();
    let grid = world.resource::<Map>().nav_grid();
    for (x, y) in [(5, 5), (6, 5), (5, 6), (6, 6)] {
        assert!(
            grid.is_claimed_by(utils::GROUND, CellPos::new(x, y)),
            "the footprint cell ({x}, {y}) is unclaimed"
        );
    }
    for (x, y) in [(4, 5), (7, 5), (5, 4), (5, 7)] {
        assert!(
            !grid.is_claimed_by(utils::GROUND, CellPos::new(x, y)),
            "the neighbor cell ({x}, {y}) is claimed past the footprint"
        );
    }
}

#[test]
fn mixed_size_bodies_push_apart() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    // A soldier spawned inside a resting wagon's footprint: contact runs
    // center to center — the wagon's circle sits half its size past the
    // anchor, deeper in than a same-anchor small body's — so the pair parts
    // by the sum of their radii instead of missing the overlap or phantom
    // pushing on one side.
    let (wagon, _) = utils::spawn_owned(&mut app, "wagon", 5, 5, 0);
    // Spawned clear — placement rightly refuses the claimed footprint — and
    // moved into the overlap directly; the claim plane re-derives from the
    // bodies either way.
    let (soldier, _) = utils::spawn_owned(&mut app, "soldier", 9, 9, 0);
    app.world_mut()
        .entity_mut(soldier)
        .get_mut::<LocationComponent>()
        .unwrap()
        .position = utils::pos(6, 6);

    utils::run_ticks(&mut app, 300);

    let world = app.world_mut();
    let wagon_center = body::center(utils::position_of(world, wagon), CellSize::new(2, 2));
    let soldier_center = body::center(utils::position_of(world, soldier), CellSize::ONE);
    let separation = wagon_center.distance(soldier_center);
    assert!(
        separation >= FixedU64::from_num(1.49),
        "centers must part by the radii sum 1.5: separation {separation}"
    );
    // The exact equilibrium: the pair parts along the diagonal through both
    // centers, each carried half the overlap per pass until contact clears.
    assert_eq!(
        utils::position_of(world, wagon),
        utils::position_bits(0x4_b83c_499a, 0x4_b83c_499a)
    );
    assert_eq!(
        utils::position_of(world, soldier),
        utils::position_bits(0x6_47c3_b666, 0x6_47c3_b666)
    );
}

#[test]
fn wide_walk_reaches_goal() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (wagon, id) = utils::spawn_owned(&mut app, "wagon", 2, 2, 0);

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
fn claims_are_rebuilt_from_bodies() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    let (soldier, id) = utils::spawn_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 2),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 200);

    // The claim plane is derived from the settled bodies: the cell under
    // the body is claimed, the spawn cell long released.
    let world = app.world_mut();
    let cell = utils::cell_of(world, soldier);
    assert_eq!(cell, CellPos::new(12, 2));
    assert!(
        world
            .resource::<Map>()
            .nav_grid()
            .is_claimed_by(utils::GROUND, cell)
    );
    assert!(
        !world
            .resource::<Map>()
            .nav_grid()
            .is_claimed_by(utils::GROUND, CellPos::new(2, 2))
    );
}
