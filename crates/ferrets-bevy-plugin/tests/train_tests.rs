//! Train order: spawning units, charging the price, rejecting invalid orders,
//! and dropping a queued entry back to its owner on a cancel.

mod utils;

use bevy::prelude::{App, World};
use ferrets_content::player_stats::PlayerStatId;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{rise, train::TrainQueueComponent},
    map::Map,
    movement_model::MovementModel,
    player_stats::PlayerStats,
    resources::PlayerResources,
    simulation_id::SimulationId,
    supply,
};

#[test]
fn train_spawns_units_and_deducts_cost() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        utils::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 100);

    for _ in 0..3 {
        utils::push_command(
            &mut app,
            PlayerCommand::TrainEntity {
                trainer: barracks_id,
                type_name: "soldier".into(),
            },
        );
    }

    utils::run_ticks(&mut app, 14);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 3);

    let world = app.world_mut();
    // 3 × 30 gold paid; the fourth order was unaffordable and ignored.
    assert_eq!(utils::gold(world), 10);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );
    // The unaffordable order never produces a fourth soldier.
    utils::run_ticks(&mut app, 30);
    assert!(utils::count_of_type(app.world_mut(), "soldier") <= 3);
    assert_eq!(utils::gold(app.world_mut()), 10);

    // A trainable type outside the barracks' catalogue is rejected without payment.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "worker".into(),
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::count_of_type(app.world_mut(), "worker"), 0);
    assert_eq!(utils::gold(app.world_mut()), 10);

    // Every trained unit appeared adjacent to the barracks footprint.
    let world = app.world_mut();
    for soldier in utils::owned_of_type(world, "soldier", 0) {
        utils::assert_adjacent_to_footprint(world, soldier, barracks);
    }
}

#[test]
fn trained_unit_stands_only_on_terrain_its_domain_takes() {
    // A 2×2 barracks at (10,10) in a ring of water but for (9,10). The
    // soldier's domain takes every terrain, so it takes the ring's first cell:
    // the eight cells beside the footprint's sides all sit 1² + 3² = 10
    // half-cells² from its middle, and the tie goes to the lowest row, then
    // column — (10,9). The landlubber's domain takes land alone: (9,10).
    let ring: Vec<(u32, u32)> = (9..=12)
        .flat_map(|y| (9..=12).map(move |x| (x, y)))
        .filter(|&(x, y)| !(10..=11).contains(&x) || !(10..=11).contains(&y))
        .filter(|&cell| cell != (9, 10))
        .collect();
    let mut app = utils::dry_orders_app(MovementModel::Cell, &ring, |registry, _| {
        let barracks = registry
            .entity("barracks")
            .expect("the orders content declares a barracks")
            .clone();
        let mut camp = barracks.with_trainer(["soldier", "landlubber"]);
        camp.name = "camp".to_string();
        registry.register(camp);
    });
    let world = app.world_mut();
    let (_, camp_id) = utils::create_entity(world, "camp", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 60);

    for type_name in ["soldier", "landlubber"] {
        utils::push_command(
            &mut app,
            PlayerCommand::TrainEntity {
                trainer: camp_id,
                type_name: type_name.into(),
            },
        );
    }
    utils::run_ticks(&mut app, 30);

    let world = app.world_mut();
    assert_eq!(cells_of(world, "soldier"), [CellPos::new(10, 9)]);
    assert_eq!(cells_of(world, "landlubber"), [CellPos::new(9, 10)]);
}

#[test]
fn rising_building_refuses_training() {
    let mut app = utils::orders_app();
    let (barracks, barracks_id) = utils::create_crewed_rising(&mut app, "barracks", 10, 10, 0);
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );

    // The order is refused outright: nothing queued, nothing paid.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 0);
    assert_eq!(utils::gold(app.world_mut()), 30);
    assert!(utils::order_queue_is_empty(app.world_mut(), barracks));

    // Having risen lifts the restriction.
    rise::mark_as_risen(app.world_mut(), barracks);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );
    utils::run_ticks(&mut app, 14);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);
    assert_eq!(utils::gold(app.world_mut()), 0);
}

#[test]
fn own_soldiers_around_trainer_make_way_for_trained_one() {
    // Walls everywhere but the barracks and a corridor running east from it
    // along row 10, and the barracks' own idle soldiers fill the corridor
    // from (12, 10) to (19, 10) — every cell the release searches. The
    // finished soldier finds no cell, so the one on (12, 10) is asked off;
    // the soldiers ahead of it step along the corridor in turn, and the new
    // one stands on the seventh tick.
    let mut app = utils::orders_app();
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
        }
        for x in 12..=30 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 10), false);
        }
    }
    let (_, barracks_id) =
        utils::create_entity(app.world_mut(), "barracks", utils::pos(10, 10), Some(0)).unwrap();
    for x in 12..=19 {
        utils::create_owned(&mut app, "soldier", x, 10, 0);
    }
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", 100);
    queue_soldier(&mut app, barracks_id);

    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 8);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 9);
}

//
// ─── Canceling ─────────────────────────────────────────────────────────────
//

#[test]
fn cancel_train_refunds_queued_unit() {
    let mut app = utils::orders_app();
    let (barracks, barracks_id) = utils::create_owned(&mut app, "barracks", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    for _ in 0..3 {
        queue_soldier(&mut app, barracks_id);
    }
    // The cancel rides in the same batch, so it lands before the first
    // soldier's four ticks are up and the slots still number three.
    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: barracks_id,
            slot: 2,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // 100 granted − 3 × 30 paid + 30 back for the dropped soldier.
    assert_eq!(utils::gold(app.world_mut()), 40);
    assert_eq!(utils::train_queue_len(app.world(), barracks), 2);

    // The two that remain still arrive, four ticks each.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 2);
}

#[test]
fn cancel_train_drops_unit_in_progress_and_restarts_next() {
    let mut app = utils::orders_app();
    let (academy, academy_id) = utils::create_owned(&mut app, "academy", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    queue_recruit(&mut app, academy_id);
    queue_recruit(&mut app, academy_id);
    // Ten of the first recruit's twenty ticks are worked before it is dropped.
    utils::run_ticks(&mut app, utils::APPLY + 10);
    // 100 granted − 2 × 30.
    assert_eq!(utils::gold(app.world_mut()), 40);

    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: academy_id,
            slot: 0,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    // 40 + the 30 the dropped recruit cost.
    assert_eq!(utils::gold(app.world_mut()), 70);
    assert_eq!(utils::train_queue_len(app.world(), academy), 1);

    // The one behind it starts over: the ticks already worked belonged to the
    // recruit that was dropped. Its first tick of work is the one the cancel
    // landed on, so 18 more still leave it a tick short of its 20.
    utils::run_ticks(&mut app, 18);
    assert_eq!(utils::count_of_type(app.world_mut(), "recruit"), 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "recruit"), 1);
}

#[test]
fn cancel_train_past_queue_end_changes_nothing() {
    let mut app = utils::orders_app();
    let (barracks, barracks_id) = utils::create_owned(&mut app, "barracks", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    queue_soldier(&mut app, barracks_id);
    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: barracks_id,
            slot: 7,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // 100 granted − 30 for one soldier, and nothing paid back.
    assert_eq!(utils::gold(app.world_mut()), 70);
    assert_eq!(utils::train_queue_len(app.world(), barracks), 1);

    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "soldier"), 1);
}

#[test]
fn cancel_train_ignores_rival_trainer() {
    let mut app = utils::orders_app();
    // A rival's barracks with one soldier queued, seated straight onto it.
    let (barracks, barracks_id) = utils::create_owned(&mut app, "barracks", 14, 14, 1);
    app.world_mut()
        .get_mut::<TrainQueueComponent>(barracks)
        .expect("trainers carry a production queue")
        .0
        .push_back("soldier".into());
    utils::grant_gold(&mut app, 100);

    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: barracks_id,
            slot: 0,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::train_queue_len(app.world(), barracks),
        1,
        "a player cancels only its own production"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        100,
        "and is paid nothing for a rival's"
    );
}

#[test]
fn cancel_train_of_waiting_slot_keeps_progress_of_entry_in_work() {
    let mut app = utils::orders_app();
    let (academy, academy_id) = utils::create_owned(&mut app, "academy", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    queue_recruit(&mut app, academy_id);
    queue_recruit(&mut app, academy_id);
    // Ten of the first recruit's twenty ticks are worked.
    utils::run_ticks(&mut app, utils::APPLY + 10);
    // 100 granted − 2 × 30.
    assert_eq!(utils::gold(app.world_mut()), 40);

    // Slot 1 is the one waiting its turn, so the progress counter belongs to
    // the entry that stays.
    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: academy_id,
            slot: 1,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    // 40 + the 30 the dropped recruit cost.
    assert_eq!(utils::gold(app.world_mut()), 70);
    assert_eq!(utils::train_queue_len(app.world(), academy), 1);

    // Untouched progress: thirteen ticks are worked by now, so seven more
    // finish the twenty and no further recruit follows.
    utils::run_ticks(&mut app, 7);
    assert_eq!(utils::count_of_type(app.world_mut(), "recruit"), 1);
    utils::run_ticks(&mut app, 25);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "recruit"),
        1,
        "the dropped entry never arrives"
    );
}

#[test]
fn cancel_train_releases_supply_of_dropped_entry() {
    let mut app = utils::supply_app();
    let (_, lodge_id) = utils::create_owned(&mut app, "lodge", 10, 10, 0);
    utils::create_owned(&mut app, "camp", 20, 20, 0);
    app.world_mut().resource_mut::<PlayerStats>().set_base(
        0,
        PlayerStatId::MAX_SUPPLY,
        FixedU64::from_num(2),
    );
    utils::grant_gold(&mut app, 100);

    // Two settlers at a supply of one each fill the ceiling of two:
    // 100 − 2 × 10 paid.
    utils::train_settler(&mut app, lodge_id);
    utils::train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
    assert_eq!(utils::gold(app.world()), 80);

    // A third is refused at the cap: nothing queued, nothing paid.
    utils::train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
    assert_eq!(utils::gold(app.world()), 80);

    // The dropped entry gives its reservation back with its 10 gold: one
    // supply in use, 80 + 10 = 90.
    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: lodge_id,
            slot: 1,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ONE);
    assert_eq!(utils::gold(app.world()), 90);

    // The room it left admits the third where the cap refused it: 90 − 10 =
    // 80, and two in use again — the first settler's, queued or already
    // standing, and the new entry's.
    utils::train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
    assert_eq!(utils::gold(app.world()), 80);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// The cells player 0's units of `type_name` stand on.
fn cells_of(world: &mut World, type_name: &str) -> Vec<CellPos> {
    utils::owned_of_type(world, type_name, 0)
        .into_iter()
        .map(|unit| utils::cell_of(world, unit))
        .collect()
}

/// Queues one soldier on `trainer`.
fn queue_soldier(app: &mut App, trainer: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: "soldier".into(),
        },
    );
}

/// Queues one recruit on `trainer`.
fn queue_recruit(app: &mut App, trainer: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: "recruit".into(),
        },
    );
}
