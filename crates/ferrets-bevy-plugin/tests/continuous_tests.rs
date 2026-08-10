//! The continuous movement model: free positions, bodies pushing apart, and
//! terrain-checked commits.
//!
//! Rest positions and cells are asserted exactly where they are pinned:
//! every value is deterministic fixed-point math replayed identically on
//! every lockstep peer, so a drifted equilibrium — however plausible — is
//! a desync. Changing a pinned value must be a conscious decision.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_physics::body::center_cell;
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
    assert_eq!(
        world
            .entity(soldier)
            .get::<LocationComponent>()
            .unwrap()
            .position,
        utils::pos(20, 14)
    );
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
    let position_of = |entity: bevy::prelude::Entity| {
        world
            .entity(entity)
            .get::<LocationComponent>()
            .unwrap()
            .position
    };
    let a = position_of(first.0);
    let b = position_of(second.0);
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
            map.nav_grid_mut()
                .set_occupied_by(utils::GROUND, CellPos::new(8, y), true);
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
            .map(|(soldier, _)| {
                center_cell(
                    world
                        .entity(*soldier)
                        .get::<LocationComponent>()
                        .unwrap()
                        .position,
                )
            })
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
        let position = world
            .entity(soldier)
            .get::<LocationComponent>()
            .unwrap()
            .position;
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
        let position_of = |entity: bevy::prelude::Entity| {
            world
                .entity(entity)
                .get::<LocationComponent>()
                .unwrap()
                .position
        };
        let (a, b) = (position_of(left), position_of(right));
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
            .map(|(soldier, _)| {
                app.world()
                    .entity(*soldier)
                    .get::<LocationComponent>()
                    .unwrap()
                    .position
            })
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
    let mut cells: Vec<CellPos> = settled.iter().map(|&body| center_cell(body)).collect();
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
            center_cell(
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
            map.nav_grid_mut()
                .set_occupied_by(utils::GROUND, CellPos::new(x, 4), true);
            map.nav_grid_mut()
                .set_occupied_by(utils::GROUND, CellPos::new(x, 6), true);
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
        center_cell(
            world
                .entity(idle)
                .get::<LocationComponent>()
                .unwrap()
                .position
        ),
        CellPos::new(12, 5)
    );
    assert_eq!(
        center_cell(
            world
                .entity(walker)
                .get::<LocationComponent>()
                .unwrap()
                .position
        ),
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
        .map(|&entity| {
            world
                .entity(entity)
                .get::<LocationComponent>()
                .unwrap()
                .position
        })
        .collect();
    for x in 1..13 {
        let cell = CellPos::new(x, 5);
        let expected = bodies.iter().any(|&body| center_cell(body) == cell);
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
