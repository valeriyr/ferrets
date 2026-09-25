//! Train order: spawning units, charging the price, rejecting invalid orders,
//! and dropping a queued entry back to its owner on a cancel.

mod utils;

use bevy::prelude::App;
use ferrets_content::player_stats::PlayerStatId;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        build::{SiteWork, UnderConstructionComponent},
        train::TrainQueueComponent,
    },
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
fn building_under_construction_refuses_training() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        utils::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world
        .entity_mut(barracks)
        .insert(UnderConstructionComponent {
            progress: 0,
            work: SiteWork::Crew {
                builders: Default::default(),
            },
        });
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

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

    // Construction finishing lifts the restriction.
    app.world_mut()
        .entity_mut(barracks)
        .remove::<UnderConstructionComponent>();
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
fn cancel_train_of_queued_slot_leaves_progress_of_one_in_hand() {
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
