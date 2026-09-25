//! Breeding on the engine's own content: births on a timer up to a limit,
//! broodlings seated in their breeder's berths, what happens when a broodling
//! changes form or its breeder does, and what its death or the breeder's
//! leaves behind.

mod utils;

use bevy::prelude::*;
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedU64, fixed_urect::FixedURect};
use ferrets_pathfinder::layer_mask::LayerMask;
use ferrets_simulation::{
    brood,
    command::{PlayerCommand, SelectMode},
    components::{
        attached::AttachedComponent,
        brood::{BredComponent, BroodComponent},
        build::UnderConstructionComponent,
        entity_info::EntityInfoComponent,
        order_queue::OrderQueueComponent,
        rally::{RallyPointComponent, RallyTarget},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent, SpawnCause},
    game_loop::orders::{self, Refusal},
    map::Map,
    movement_model::MovementModel,
    order::Order,
    simulation_id::SimulationId,
    spawn,
    statistics::Statistics,
    supply,
};
use utils::GROUND;

//
// ─── Breeding ──────────────────────────────────────────────────────────────────
//

#[test]
fn first_broodling_is_born_one_period_after_breeder_stands() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);

    // Period 10: nothing through tick 9, one on tick 10.
    utils::run_ticks(&mut app, 9);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 0);
    utils::run_ticks(&mut app, 1);
    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 1);

    let grub = grubs[0];
    let world = app.world();
    assert_eq!(entity_def::owner(world, grub), Some(0));
    assert_eq!(
        world
            .get::<AttachedComponent>(grub)
            .map(|attached| attached.job),
        Some(hatch_id)
    );
    assert_eq!(
        world.get::<BredComponent>(grub).map(|bred| bred.by),
        Some(hatch_id)
    );
    assert_eq!(broodlings_of(&app, hatch).len(), 1);
}

#[test]
fn births_stop_at_limit_and_timer_holds() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 20);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);

    // Births at 10, 20 and 30 fill the limit of three.
    utils::run_ticks(&mut app, 30);
    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 3);

    // One grub starts growing at tick 31: off the count, so the timer runs —
    // five ticks of it by tick 35.
    order_morph(&mut app, grubs[0], "worker");
    utils::run_ticks(&mut app, 5);
    assert_eq!(broodlings_of(&app, hatch).len(), 2);
    assert_eq!(progress_of(&app, hatch), 5);

    // The growth is called off at tick 36: the grub comes back to its seat and
    // the count, and the timer holds at 5 — no birth through tick 45.
    utils::soft_cancel_orders(app.world_mut(), grubs[0]);
    utils::run_ticks(&mut app, 10);
    assert_eq!(broodlings_of(&app, hatch).len(), 3);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
    assert_eq!(progress_of(&app, hatch), 5);

    // Another grub starts growing at tick 46: the timer resumes from 5 and
    // the replacement is born at tick 50 (5 + 5), not 56.
    order_morph(&mut app, grubs[1], "worker");
    utils::run_ticks(&mut app, 4);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
    assert_eq!(progress_of(&app, hatch), 0);
}

#[test]
fn births_stop_at_limit_below_slot_count() {
    // Four seats, a limit of three: the fourth seat is never filled by a birth.
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);

    utils::run_ticks(&mut app, 60);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
}

#[test]
fn breeder_under_construction_counts_no_time() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let (_, digger) = utils::create_owned(&mut app, "digger", 5, 5, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: digger,
            type_name: "ready_hatch".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );

    // Nothing is born while the site goes up; the two owed grubs are born the
    // tick it stands, the third one period later.
    let mut completed_at = None;
    for tick in 1..=80 {
        utils::run_ticks(&mut app, 1);
        let standing = alive_of_type(&mut app, "ready_hatch")
            .into_iter()
            .any(|hatch| {
                !app.world()
                    .entity(hatch)
                    .contains::<UnderConstructionComponent>()
            });
        if standing {
            completed_at = Some(tick);
            break;
        }
        assert_eq!(alive_of_type(&mut app, "grub").len(), 0, "tick {tick}");
    }
    let completed_at = completed_at.expect("the digger finishes the hatch within 80 ticks");
    assert_eq!(
        alive_of_type(&mut app, "grub").len(),
        2,
        "tick {completed_at}"
    );
    // The timer runs from the completion tick, so the third comes nine
    // ticks later.
    utils::run_ticks(&mut app, 8);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
}

#[test]
fn initial_broodlings_are_owed_at_first_breeding_tick() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "ready_hatch", 10, 10, 0);

    // Two owed grubs on the first tick; the third at tick 10, from a timer
    // that ran from the first tick.
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 8);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
}

#[test]
fn period_read_from_breeder_stat() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "stat_hatch", 10, 10, 0);

    // `brood_period` is 6 on the stat hatch.
    utils::run_ticks(&mut app, 5);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 1);
}

#[test]
fn berth_beyond_map_edge_is_held_at_edge_cell() {
    // The map is 32 cells tall; a hatch on its last three rows has its berth
    // row at y 32.5, off the map, so the seat is held at the edge cell's
    // middle, 31.5, and the one-cell grub's corner sits at 31.
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 29, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    assert_eq!(
        entity_def::position(app.world(), grub).y,
        FixedU64::from_num(31)
    );
}

#[test]
fn broodlings_take_breeder_owner_and_announce_bred() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);

    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);
    assert_eq!(entity_def::owner(app.world(), grub), Some(0));
    assert!(app.world().resource::<utils::Announced>().0.contains(
        &SimulationEvent::EntitySpawned {
            entity: grub_id,
            cause: SpawnCause::Bred { by: hatch_id },
        }
    ));
}

//
// ─── Seating and behaviour ─────────────────────────────────────────────────────
//

#[test]
fn seated_broodlings_hold_no_cells() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    let cell = utils::cell_of(app.world(), grub);
    let map = app.world().resource::<Map>();
    assert!(map.nav_grid().is_passable_by(LayerMask::from(GROUND), cell));
    assert!(!map.nav_grid().is_claimed_by(LayerMask::from(GROUND), cell));
}

#[test]
fn berths_may_ring_footprint() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    // The brood berths lie on the row below the 3×3 footprint at (10, 10);
    // every point is one cell from the footprint, so the first seat wins the
    // tie: point (10.5, 13.5), a one-cell body's corner at (10, 13).
    assert_eq!(utils::cell_of(app.world(), grub), CellPos::new(10, 13));
}

#[test]
fn three_broodlings_spread_over_five_points() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 30);

    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 3);
    // Each birth takes the lowest free point, and the roaming stance walks
    // the earlier grubs on to free points between births: by tick 30 the
    // three hold points 0, 2 and 3 of the five, pinned against the stance's
    // hashed walk.
    let mut seats: Vec<usize> = grubs
        .iter()
        .map(|&grub| app.world().get::<AttachedComponent>(grub).unwrap().seat)
        .collect();
    seats.sort_unstable();
    assert_eq!(seats, vec![0, 2, 3]);
}

#[test]
fn broodlings_roam_deterministically() {
    let positions = |model| {
        let mut app = utils::brood_app(model);
        utils::place(&mut app, "hatch", 10, 10, 0);
        utils::run_ticks(&mut app, 60);
        let grubs = alive_of_type(&mut app, "grub");
        grubs
            .iter()
            .map(|&grub| utils::position_of(app.world(), grub))
            .collect::<Vec<_>>()
    };
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        assert_eq!(positions(model), positions(model));
    }
}

#[test]
fn broodlings_are_boxed_like_units() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 30);

    // A box over the brood row takes the three grubs.
    utils::push_command(
        &mut app,
        PlayerCommand::SelectByRect {
            rect: FixedURect::new(utils::pos(9, 13), utils::pos(14, 14)),
            mode: SelectMode::Replace,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::selection(&app).len(), 3);
}

#[test]
fn broodlings_without_speed_refuse_move() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    let order = Order::Move {
        target: utils::pos(20, 20),
        size: CellSize::ONE,
        range: 0,
    };
    assert_eq!(
        orders::can_start(app.world(), grub, &order),
        Err(Refusal::Incapable)
    );
}

//
// ─── A broodling's change of form ──────────────────────────────────────────────
//

#[test]
fn broodling_steps_out_of_footprint_before_changing() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 1);

    // The egg stands on a cell of its own, off the seat, and no longer counts:
    // the grub sat at seat 0, point (10.5, 13.5), and the cell under it,
    // (10, 13), takes the egg.
    assert_eq!(type_name_of(&app, grub), "egg");
    assert!(app.world().get::<AttachedComponent>(grub).is_none());
    assert_eq!(utils::cell_of(app.world(), grub), CellPos::new(10, 13));
    assert_eq!(broodlings_of(&app, hatch).len(), 0);
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn interim_form_holds_ground_its_own_solidity_says() {
    // A passable grub grows inside a solid egg: the ground the entity holds
    // while changing is the interim form's, not the origin's — the cell it
    // was set down on is statically occupied from the tick the egg is worn.
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 1);
    let cell = utils::cell_of(app.world(), grub);
    let map = app.world().resource::<Map>();
    assert!(map.nav_grid().is_statically_occupied_by(GROUND, cell));
}

#[test]
fn step_out_searches_ground_interim_form_fits() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    // A worker stands on the grub's cell: a passable grub shares it, an egg
    // cannot, so the egg is laid on the next free cell.
    let cell = utils::cell_of(app.world(), grub);
    utils::create_owned(&mut app, "worker", cell.x, cell.y, 0);
    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 1);

    // The worker holds (10, 13); the search around it takes the free cell
    // nearest that middle, (9, 13) — (10, 12) above it is the hatch's own
    // ground.
    assert_eq!(type_name_of(&app, grub), "egg");
    assert_eq!(utils::cell_of(app.world(), grub), CellPos::new(9, 13));
}

#[test]
fn broodling_hemmed_in_is_refused_no_room() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cells(&mut app, CellPos::new(10, 10), CellSize::new(3, 3));
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    let order = Order::Morph {
        type_name: "worker".into(),
    };
    assert_eq!(
        orders::can_start(app.world(), grub, &order),
        Err(Refusal::NoRoom)
    );
}

#[test]
fn landing_takes_cell_interim_form_stood_on() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 1);
    let egg_cell = utils::cell_of(app.world(), grub);
    utils::run_ticks(&mut app, 10);

    assert_eq!(type_name_of(&app, grub), "worker");
    assert_eq!(utils::cell_of(app.world(), grub), egg_cell);
    assert!(app.world().get::<BredComponent>(grub).is_none());
}

#[test]
fn nearby_landing_moves_form_that_does_not_fit_where_it_grew() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    // A 3×3 recentred on the egg's cell would overlap the hatch; it lands on
    // the nearest ground that takes it instead.
    order_morph(&mut app, grub, "bulk");
    utils::run_ticks(&mut app, 11);

    // The egg stood on (10, 13); a 3×3 anchored there covers rows 13–15,
    // clear of the hatch, so the search takes it as it is.
    assert_eq!(type_name_of(&app, grub), "bulk");
    assert_eq!(utils::cell_of(app.world(), grub), CellPos::new(10, 13));
}

#[test]
fn nearby_change_fizzles_and_refunds_when_nothing_is_free() {
    let mut app = utils::brood_app(MovementModel::Cell);
    // Room for the hatch and one egg, and for nothing three cells wide.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cells(&mut app, CellPos::new(10, 10), CellSize::new(3, 3));
    free_cells(&mut app, CellPos::new(11, 13), CellSize::ONE);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "bulk");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, grub), "egg");
    utils::run_ticks(&mut app, 10);

    // Back to a grub in its seat, with the price returned, beside the grub
    // born while it grew.
    assert_eq!(type_name_of(&app, grub), "grub");
    assert_eq!(
        app.world()
            .get::<AttachedComponent>(grub)
            .map(|attached| attached.job),
        Some(hatch_id)
    );
    assert_eq!(broodlings_of(&app, hatch).len(), 2);
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn fizzled_change_dies_when_declared_to() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    // Room for the hatch and one egg, and for nothing three cells wide.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cells(&mut app, CellPos::new(10, 10), CellSize::new(3, 3));
    free_cells(&mut app, CellPos::new(11, 13), CellSize::ONE);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);

    order_morph(&mut app, grub, "hulk");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, grub), "egg");
    utils::run_ticks(&mut app, 10);

    // The landing found no ground; the transition dies rather than reverts,
    // the egg is what died, and the price came back all the same.
    assert_eq!(utils::gold(app.world()), 10);
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied { entity, cause: DeathCause::Canceled, entity_type, .. }
                    if *entity == grub_id && *entity_type == type_id(&app, "egg")
            ))
    );
    assert_eq!(broodlings_of(&app, hatch).len(), 1);
    utils::run_ticks(&mut app, 2);
    utils::assert_despawned(app.world_mut(), grub);
}

#[test]
fn landed_broodling_obeys_breeder_rally() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 11);

    assert_eq!(type_name_of(&app, grub), "worker");
    assert_eq!(move_targets(&app, grub), vec![utils::pos(20, 20)]);
}

#[test]
fn breeder_takes_rally_point_only_when_its_broodlings_produce() {
    let mut app = utils::brood_app(MovementModel::Cell);
    // Grubs grow into workers as production; a piglet's change into a worker
    // is declared a change, so the pen produces nothing.
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let pen = utils::place(&mut app, "linger_pen", 20, 10, 0);

    assert_eq!(
        app.world()
            .get::<RallyPointComponent>(hatch)
            .map(|rally| rally.0),
        Some(None)
    );
    assert!(app.world().get::<RallyPointComponent>(pen).is_none());
}

#[test]
fn refused_reserving_change_leaves_broodling_seated() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    // Every cell blocked but the hatch's own and one beneath its berths: a
    // grub has a cell to step onto, but the 3x3 mound it would reserve does
    // not fit anywhere around it.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cells(&mut app, CellPos::new(10, 10), CellSize::new(3, 3));
    free_cells(&mut app, CellPos::new(11, 13), CellSize::ONE);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    // Judged before anything moves: the grub keeps its seat and its count,
    // and the gold stays.
    assert_eq!(
        orders::can_start(
            app.world(),
            grub,
            &Order::Morph {
                type_name: "mound".into()
            }
        ),
        Err(Refusal::NoRoom)
    );
    order_morph(&mut app, grub, "mound");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, grub), "grub");
    assert!(app.world().get::<AttachedComponent>(grub).is_some());
    assert_eq!(
        broodlings_of(&app, hatch),
        vec![entity_def::simulation_id(app.world(), grub)]
    );
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn egg_takes_rally_point_and_broodling_does_not() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);
    assert!(app.world().get::<RallyPointComponent>(grub).is_none());

    // A grub committed to nothing takes no rally point.
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: grub_id,
            target: Some(RallyTarget::Position(utils::pos(25, 25))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<RallyPointComponent>(grub).is_none());

    // The egg it grows a worker in does.
    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, grub), "egg");
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: grub_id,
            target: Some(RallyTarget::Position(utils::pos(25, 25))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        app.world()
            .get::<RallyPointComponent>(grub)
            .map(|rally| rally.0),
        Some(Some(RallyTarget::Position(utils::pos(25, 25))))
    );
}

#[test]
fn hatched_unit_obeys_egg_rally_over_breeder_rally() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 3);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: grub_id,
            target: Some(RallyTarget::Position(utils::pos(25, 25))),
        },
    );
    utils::run_ticks(&mut app, 8);

    // The egg's rally is gone with the egg; its target sent the worker on.
    assert_eq!(type_name_of(&app, grub), "worker");
    assert!(app.world().get::<RallyPointComponent>(grub).is_none());
    assert_eq!(move_targets(&app, grub), vec![utils::pos(25, 25)]);
}

#[test]
fn reverted_egg_forgets_rally_point() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 3);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: grub_id,
            target: Some(RallyTarget::Position(utils::pos(25, 25))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::soft_cancel_orders(app.world_mut(), grub);
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, grub), "grub");
    assert!(app.world().get::<RallyPointComponent>(grub).is_none());
}

#[test]
fn canceled_growth_reseats_when_seat_is_free() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, grub), "egg");
    utils::soft_cancel_orders(app.world_mut(), grub);
    utils::run_ticks(&mut app, 1);

    // The seat again, the price back, and the supply the worker held let go.
    assert_eq!(type_name_of(&app, grub), "grub");
    assert_eq!(
        app.world()
            .get::<AttachedComponent>(grub)
            .map(|attached| attached.job),
        Some(hatch_id)
    );
    assert_eq!(broodlings_of(&app, hatch).len(), 1);
    assert_eq!(utils::gold(app.world()), 10);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ZERO);
}

#[test]
fn canceled_growth_dies_when_no_seat_is_free() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    utils::grant_gold(&mut app, 10);
    // Two seats, a limit of two; the den unlocks the twenty-tick brute.
    let hatch = utils::place(&mut app, "tight_hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::place(&mut app, "den", 20, 20, 0);
    utils::run_ticks(&mut app, 20);
    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 2);

    // One grub grows from tick 21; its replacement is born at tick 30 and
    // takes the seat it left, so the growth called off at tick 31 finds none.
    order_morph(&mut app, grubs[0], "brute");
    utils::run_ticks(&mut app, 10);
    assert_eq!(type_name_of(&app, grubs[0]), "egg");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);

    utils::soft_cancel_orders(app.world_mut(), grubs[0]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    assert_eq!(broodlings_of(&app, hatch).len(), 2);
    let grub_id = entity_def::simulation_id(app.world(), grubs[0]);
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied { entity, cause: DeathCause::Unseated { of }, .. }
                    if *entity == grub_id && *of == hatch_id
            ))
    );
    // The growth was refunded all the same.
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn canceled_growth_dies_when_breeder_is_gone() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 2);
    spawn::destroy_entity(app.world_mut(), hatch);
    utils::run_ticks(&mut app, 1);
    // The egg is no broodling: it outlives the hatch.
    assert_eq!(type_name_of(&app, grub), "egg");

    utils::soft_cancel_orders(app.world_mut(), grub);
    utils::run_ticks(&mut app, 1);
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied { entity, cause: DeathCause::Unseated { of }, .. }
                    if *entity == grub_id && *of == hatch_id
            ))
    );
}

#[test]
fn change_refused_when_supply_headroom_is_short() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 40);
    // The hatch provides 3 supply; the den unlocks the brute.
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::place(&mut app, "den", 20, 20, 0);
    utils::run_ticks(&mut app, 30);
    let grubs = alive_of_type(&mut app, "grub");

    // Two workers take 2 of the 3; a brute at 2 more does not fit, a worker at
    // 1 more does.
    order_morph(&mut app, grubs[0], "worker");
    order_morph(&mut app, grubs[1], "worker");
    utils::run_ticks(&mut app, 1);
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
    let brute = Order::Morph {
        type_name: "brute".into(),
    };
    let worker = Order::Morph {
        type_name: "worker".into(),
    };
    assert_eq!(
        orders::can_start(app.world(), grubs[2], &brute),
        Err(Refusal::NoSupply)
    );
    assert_eq!(orders::can_start(app.world(), grubs[2], &worker), Ok(()));
}

#[test]
fn changing_entity_counts_destination_supply() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::place(&mut app, "den", 20, 20, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    assert_eq!(supply::used(app.world(), 0), FixedU64::ZERO);

    // The egg holds the brute's 2 from the tick the growth starts.
    order_morph(&mut app, grub, "brute");
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, grub), "egg");
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
}

#[test]
fn canceled_change_dies_when_declared_to() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    let grub_id = entity_def::simulation_id(app.world(), grub);

    order_morph(&mut app, grub, "flit");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, grub), "egg");
    utils::soft_cancel_orders(app.world_mut(), grub);
    utils::run_ticks(&mut app, 1);

    // The price came back, and the egg is what died.
    assert_eq!(utils::gold(app.world()), 10);
    assert_eq!(broodlings_of(&app, hatch).len(), 0);
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied { entity, cause: DeathCause::Canceled, entity_type, .. }
                    if *entity == grub_id && *entity_type == type_id(&app, "egg")
            ))
    );
    utils::run_ticks(&mut app, 2);
    utils::assert_despawned(app.world_mut(), grub);
}

//
// ─── The breeder dies ──────────────────────────────────────────────────────────
//

#[test]
fn perish_kills_broodlings_with_breeder() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 30);
    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 3);

    spawn::destroy_entity(app.world_mut(), hatch);
    utils::run_ticks(&mut app, 1);

    assert_eq!(alive_of_type(&mut app, "grub").len(), 0);
    let orphaned = app
        .world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter(|event| {
            matches!(
                event,
                SimulationEvent::EntityDied { cause: DeathCause::Orphaned { of }, .. } if *of == hatch_id
            )
        })
        .count();
    assert_eq!(orphaned, 3);
}

#[test]
fn linger_sets_broodlings_down_beside_ruin() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "linger_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);

    spawn::destroy_entity(app.world_mut(), pen);
    utils::run_ticks(&mut app, 3);

    // Both piglets step out of their berths onto free cells and stand alone.
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);
    for piglet in piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_none());
        assert!(entity_def::stands_on_grid(app.world(), piglet));
    }
}

#[test]
fn changing_broodling_outlives_breeder_and_lands_without_rally() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];

    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 2);
    spawn::destroy_entity(app.world_mut(), hatch);
    utils::run_ticks(&mut app, 10);

    assert_eq!(type_name_of(&app, grub), "worker");
    assert!(entity_def::orders(app.world(), grub).is_empty());
}

//
// ─── The breeder changes form ──────────────────────────────────────────────────
//

#[test]
fn breeder_changes_form_with_broodlings_seated() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 30);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);

    let order = Order::Morph {
        type_name: "great_hatch".into(),
    };
    assert_eq!(orders::can_start(app.world(), hatch, &order), Ok(()));
    order_morph(&mut app, hatch, "great_hatch");
    utils::run_ticks(&mut app, 12);

    assert_eq!(type_name_of(&app, hatch), "great_hatch");
    let grubs = alive_of_type(&mut app, "grub");
    assert_eq!(grubs.len(), 3);
    for grub in grubs {
        assert_eq!(
            app.world()
                .get::<AttachedComponent>(grub)
                .map(|attached| attached.job),
            Some(hatch_id)
        );
    }
    assert_eq!(broodlings_of(&app, hatch).len(), 3);
}

#[test]
fn breeder_keeps_breeding_through_change_without_interim_form() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 25);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);

    // Ten ticks of change from tick 26, the hatch still worn: the third grub
    // is born at tick 30 as if nothing were happening.
    order_morph(&mut app, hatch, "bare_hatch");
    utils::run_ticks(&mut app, 4);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
    assert_eq!(type_name_of(&app, hatch), "hatch");
}

#[test]
fn breeder_breeds_nothing_while_wearing_interim_form() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 25);
    assert_eq!(progress_of(&app, hatch), 5);

    // The shell is worn from tick 26 to 34 and breeds nothing, the timer
    // holding at 5; the great hatch stands from tick 35 and counts on from
    // there — the third grub comes at tick 39 (5 + 4 + 1).
    order_morph(&mut app, hatch, "great_hatch");
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, hatch), "shell");
    assert_eq!(broodlings_of(&app, hatch).len(), 2);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    for grub in alive_of_type(&mut app, "grub") {
        assert_eq!(
            app.world()
                .get::<AttachedComponent>(grub)
                .map(|attached| attached.job),
            Some(hatch_id)
        );
    }
    assert_eq!(progress_of(&app, hatch), 5);
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, hatch), "great_hatch");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 3);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);
}

#[test]
fn landed_form_tops_brood_up_to_its_initial() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 12);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 1);

    // The great hatch opens with two: one grub alive at the landing, so one
    // more is owed and born the tick it stands; the timer counts on from 2.
    order_morph(&mut app, hatch, "great_hatch");
    utils::run_ticks(&mut app, 9);
    assert_eq!(type_name_of(&app, hatch), "shell");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, hatch), "great_hatch");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    assert_eq!(progress_of(&app, hatch), 3);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
}

#[test]
fn breeder_rally_point_survives_interim_form() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The shell breeds nothing, so it wants no rally point of its own; the
    // hatch's is live state and rides through to the great hatch.
    order_morph(&mut app, hatch, "great_hatch");
    utils::run_ticks(&mut app, 5);
    assert_eq!(type_name_of(&app, hatch), "shell");
    let rally = |app: &bevy::prelude::App| {
        app.world()
            .get::<RallyPointComponent>(hatch)
            .map(|rally| rally.0)
    };
    assert_eq!(
        rally(&app),
        Some(Some(RallyTarget::Position(utils::pos(20, 20))))
    );
    utils::run_ticks(&mut app, 6);
    assert_eq!(type_name_of(&app, hatch), "great_hatch");
    assert_eq!(
        rally(&app),
        Some(Some(RallyTarget::Position(utils::pos(20, 20))))
    );
}

#[test]
fn changed_breeder_is_not_sent_to_its_own_rally_point() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // A change of form makes no unit: the rally point sends nobody, least of
    // all the building itself.
    order_morph(&mut app, hatch, "bare_hatch");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, hatch), "bare_hatch");
    assert!(entity_def::orders(app.world(), hatch).is_empty());
}

#[test]
fn egg_hatching_while_interim_form_is_worn_obeys_breeder_rally() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 10);
    let grub = alive_of_type(&mut app, "grub")[0];
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hatch_id,
            target: Some(RallyTarget::Position(utils::pos(20, 20))),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // The grub's growth lands eleven ticks on; the hatch starts its change
    // three ticks later and wears the shell across that landing, its rally
    // point riding along, so the worker is still sent where the hatch said.
    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 3);
    order_morph(&mut app, hatch, "great_hatch");
    utils::run_ticks(&mut app, 8);

    assert_eq!(type_name_of(&app, hatch), "shell");
    assert_eq!(type_name_of(&app, grub), "worker");
    assert_eq!(move_targets(&app, grub), vec![utils::pos(20, 20)]);
}

#[test]
fn canceled_change_back_to_breeding_form_owes_no_top_up() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    // A great hatch opens with two grubs; one grows away, leaving one.
    let hatch = utils::place(&mut app, "great_hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    let grub = alive_of_type(&mut app, "grub")[0];
    order_morph(&mut app, grub, "worker");
    utils::run_ticks(&mut app, 1);
    assert_eq!(broodlings_of(&app, hatch).len(), 1);

    // The change toward a hatch is called off: the great hatch comes back
    // with the one grub it had, owed nothing, and the timer alone brings the
    // next one — at its ten-tick pace from the change's start.
    order_morph(&mut app, hatch, "hatch");
    utils::run_ticks(&mut app, 3);
    utils::force_cancel_orders(app.world_mut(), hatch);
    utils::run_ticks(&mut app, 1);
    assert_eq!(type_name_of(&app, hatch), "great_hatch");
    assert_eq!(broodlings_of(&app, hatch).len(), 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(broodlings_of(&app, hatch).len(), 1);
}

#[test]
fn broodlings_new_form_cannot_hold_are_destroyed() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::record_announcements(&mut app);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    let hatch_id = entity_def::simulation_id(app.world(), hatch);
    utils::run_ticks(&mut app, 30);
    assert_eq!(alive_of_type(&mut app, "grub").len(), 3);

    // Two seats on the tight hatch: one of three grubs finds none.
    order_morph(&mut app, hatch, "tight_hatch");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, hatch), "tight_hatch");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 2);
    assert_eq!(broodlings_of(&app, hatch).len(), 2);
    let unseated = app
        .world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter(|event| {
            matches!(
                event,
                SimulationEvent::EntityDied { cause: DeathCause::Unseated { of }, .. } if *of == hatch_id
            )
        })
        .count();
    assert_eq!(unseated, 1);
}

#[test]
fn linger_breeder_leaves_detached_broodlings_standing() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "linger_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    assert_eq!(alive_of_type(&mut app, "piglet").len(), 2);

    // No berths on the bare pen: both piglets are set down beside it, tie
    // cut, and the pen keeps no brood to remember them by.
    order_morph(&mut app, pen, "bare_pen");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, pen), "bare_pen");
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);
    for piglet in piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_none());
        assert!(app.world().get::<AttachedComponent>(piglet).is_none());
        assert!(entity_def::stands_on_grid(app.world(), piglet));
    }
    assert!(brood::of(app.world(), pen).is_none());
}

#[test]
fn reseat_breeder_takes_set_down_broodlings_in_when_form_holds_them_again() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    let pen_id = entity_def::simulation_id(app.world(), pen);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);

    // The seatless shell sets both down, untied, standing beside it.
    order_morph(&mut app, pen, "roomy_pen");
    utils::run_ticks(&mut app, 3);
    assert_eq!(type_name_of(&app, pen), "pen_shell");
    for &piglet in &piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_none());
        assert!(entity_def::stands_on_grid(app.world(), piglet));
    }
    assert_eq!(broodlings_of(&app, pen).len(), 0);

    // The roomy pen holds two and takes them in the tick it stands.
    utils::run_ticks(&mut app, 8);
    assert_eq!(type_name_of(&app, pen), "roomy_pen");
    for &piglet in &piglets {
        assert_eq!(
            app.world().get::<BredComponent>(piglet).map(|bred| bred.by),
            Some(pen_id)
        );
        assert_eq!(
            app.world()
                .get::<AttachedComponent>(piglet)
                .map(|attached| attached.job),
            Some(pen_id)
        );
    }
    assert_eq!(broodlings_of(&app, pen).len(), 2);
}

#[test]
fn reseat_breeder_leaves_busy_broodling() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 10);
    let pen = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");

    // One set-down piglet is ordered to change while the shell is worn: at
    // the landing it is busy with a change of its own, so only the other is
    // taken in; once it lands as a worker it is no piglet to take.
    order_morph(&mut app, pen, "roomy_pen");
    utils::run_ticks(&mut app, 3);
    order_morph(&mut app, piglets[0], "worker");
    utils::run_ticks(&mut app, 8);
    assert_eq!(type_name_of(&app, pen), "roomy_pen");
    assert!(app.world().get::<BredComponent>(piglets[0]).is_none());
    assert!(app.world().get::<BredComponent>(piglets[1]).is_some());
    assert_eq!(broodlings_of(&app, pen).len(), 1);

    utils::run_ticks(&mut app, 40);
    assert_eq!(type_name_of(&app, piglets[0]), "worker");
    assert!(app.world().get::<BredComponent>(piglets[0]).is_none());
}

#[test]
fn reseat_breeder_leaves_broodlings_beyond_its_reach() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);

    // The pen is taken off the map and its piglets linger on row 13; a pen raised at (10, 17)
    // stands four rows off, one beyond its reach of three, so it takes
    // neither.
    spawn::destroy_entity(app.world_mut(), pen);
    utils::run_ticks(&mut app, 3);
    let far = utils::place(&mut app, "reseat_pen", 10, 17, 0);
    utils::run_ticks(&mut app, 5);
    for &piglet in &piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_none());
    }
    assert!(broodlings_of(&app, far).is_empty());
}

#[test]
fn canceled_change_takes_set_down_broodlings_in_again_on_return() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");

    order_morph(&mut app, pen, "roomy_pen");
    utils::run_ticks(&mut app, 3);
    assert_eq!(broodlings_of(&app, pen).len(), 0);
    utils::force_cancel_orders(app.world_mut(), pen);
    utils::run_ticks(&mut app, 1);

    assert_eq!(type_name_of(&app, pen), "reseat_pen");
    for &piglet in &piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_some());
        assert!(app.world().get::<AttachedComponent>(piglet).is_some());
    }
    assert_eq!(broodlings_of(&app, pen).len(), 2);
}

#[test]
fn lingering_broodlings_are_taken_in_lowest_ids_first_up_to_limit() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let first = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    let second = utils::place(&mut app, "reseat_pen", 10, 15, 0);
    utils::run_ticks(&mut app, 20);
    let mut ids: Vec<SimulationId> = alive_of_type(&mut app, "piglet")
        .iter()
        .map(|&piglet| entity_def::simulation_id(app.world(), piglet))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids.len(), 4);

    // Both pens go and their piglets linger by their berths, rows 13 and
    // 18; a third pen at (13, 13) reaches all four within three cells and
    // seats two: the two lowest ids, the first pen's.
    spawn::destroy_entity(app.world_mut(), first);
    spawn::destroy_entity(app.world_mut(), second);
    utils::run_ticks(&mut app, 3);
    let third = utils::place(&mut app, "reseat_pen", 13, 13, 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(broodlings_of(&app, third), ids[..2].to_vec());
}

#[test]
fn hemmed_in_lingering_broodling_is_hidden_until_cell_frees() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cells(&mut app, CellPos::new(10, 10), CellSize::new(3, 3));
    let pen = utils::place(&mut app, "linger_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);

    // No cell around the dying pen takes a piglet — its own footprint is
    // held for the two ticks it dies — so both wait off the map, and the
    // first takes the one cell that opens, beside its berth.
    spawn::destroy_entity(app.world_mut(), pen);
    utils::run_ticks(&mut app, 1);
    utils::assert_reveal_deferred_then_lands_on(&mut app, piglets[0], CellPos::new(11, 13));
}

#[test]
fn reseat_breeder_takes_in_broodlings_another_breeder_left_behind() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let pen = utils::place(&mut app, "reseat_pen", 10, 10, 0);
    utils::run_ticks(&mut app, 20);
    let piglets = alive_of_type(&mut app, "piglet");
    assert_eq!(piglets.len(), 2);

    // The pen goes and its piglets linger on the row beneath its ruin; a
    // second pen raised flush against the ruin's east side has them within
    // its three-cell reach and two seats free, so it takes both in on its
    // first operating tick.
    spawn::destroy_entity(app.world_mut(), pen);
    utils::run_ticks(&mut app, 3);
    for &piglet in &piglets {
        assert!(app.world().get::<BredComponent>(piglet).is_none());
    }
    let other = utils::place(&mut app, "reseat_pen", 13, 10, 0);
    let other_id = entity_def::simulation_id(app.world(), other);
    utils::run_ticks(&mut app, 1);

    for &piglet in &piglets {
        assert_eq!(
            app.world().get::<BredComponent>(piglet).map(|bred| bred.by),
            Some(other_id)
        );
    }
    assert_eq!(broodlings_of(&app, other).len(), 2);
}

#[test]
fn broodlings_die_when_new_form_has_no_berths() {
    let mut app = utils::brood_app(MovementModel::Cell);
    let hatch = utils::place(&mut app, "hatch", 10, 10, 0);
    utils::run_ticks(&mut app, 30);

    order_morph(&mut app, hatch, "bare_hatch");
    utils::run_ticks(&mut app, 11);
    assert_eq!(type_name_of(&app, hatch), "bare_hatch");
    assert_eq!(alive_of_type(&mut app, "grub").len(), 0);
    assert!(app.world().get::<BroodComponent>(hatch).is_none());
}

//
// ─── Statistics ────────────────────────────────────────────────────────────────
//

#[test]
fn production_tally_counts_transitions_declared_production() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::grant_gold(&mut app, 20);
    utils::place(&mut app, "hatch", 10, 10, 0);
    utils::place(&mut app, "den", 20, 20, 0);
    utils::run_ticks(&mut app, 20);
    let grubs = alive_of_type(&mut app, "grub");

    order_morph(&mut app, grubs[0], "worker");
    order_morph(&mut app, grubs[1], "brute");
    utils::run_ticks(&mut app, 22);
    assert_eq!(type_name_of(&app, grubs[0]), "worker");
    assert_eq!(type_name_of(&app, grubs[1]), "brute");

    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).produced(type_id(&app, "worker")), 1);
    assert_eq!(statistics.player(0).produced(type_id(&app, "brute")), 0);
    assert_eq!(statistics.player(0).produced(type_id(&app, "grub")), 0);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The alive entities of `type_name`, in ascending simulation-id order.
fn alive_of_type(app: &mut App, type_name: &str) -> Vec<Entity> {
    let world = app.world();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .filter(|&entity| {
            world
                .get::<EntityInfoComponent>(entity)
                .is_some_and(|info| info.type_name() == type_name)
        })
        .collect()
}

/// The type an entity currently is.
fn type_name_of(app: &App, entity: Entity) -> String {
    app.world()
        .entity(entity)
        .get::<EntityInfoComponent>()
        .expect("a live entity carries its info")
        .type_name()
        .to_string()
}

/// The handle of a registered type.
fn type_id(app: &App, type_name: &str) -> ferrets_content::entity_type_def::EntityTypeId {
    app.world()
        .resource::<ferrets_content::registry::ContentRegistry>()
        .type_id(type_name)
        .expect("test content registers the type")
}

/// The broodlings a breeder counts.
fn broodlings_of(app: &App, breeder: Entity) -> Vec<SimulationId> {
    app.world()
        .get::<BroodComponent>(breeder)
        .map(|brood| brood.broodlings.clone())
        .unwrap_or_default()
}

/// A breeder's ticks toward its next birth.
fn progress_of(app: &App, breeder: Entity) -> u32 {
    app.world()
        .get::<BroodComponent>(breeder)
        .expect("a breeder counts a brood")
        .progress
}

/// Pushes a Morph order into `type_name` onto the entity's queue.
/// The order is prepared on the tick after this call and lands when its
/// progress runs out: a change of ten ticks stands in its new form eleven
/// ticks later, an interim form from the first. Suites that look past the
/// landing itself — at a rally, a birth — run one tick more.
fn order_morph(app: &mut App, entity: Entity, type_name: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<OrderQueueComponent>()
        .unwrap()
        .push(
            Order::Morph {
                type_name: type_name.to_string(),
            },
            None,
        );
}

/// The targets of every Move in `entity`'s queue, front first.
fn move_targets(app: &App, entity: Entity) -> Vec<ferrets_math::fixed_uvec2::FixedUVec2> {
    entity_def::orders(app.world(), entity)
        .iter()
        .filter_map(|order| match order {
            Order::Move { target, .. } => Some(*target),
            _ => None,
        })
        .collect()
}

/// Frees the static occupation of the `size` cells at `origin`.
fn free_cells(app: &mut App, origin: CellPos, size: CellSize) {
    let mut map = app.world_mut().resource_mut::<Map>();
    for dy in 0..size.height {
        for dx in 0..size.width {
            let cell = CellPos::new(origin.x + dx, origin.y + dy);
            if map.nav_grid().is_statically_occupied_by(GROUND, cell) {
                map.set_static_occupied(GROUND, cell, false);
            }
        }
    }
}
