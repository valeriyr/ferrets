//! Supply cap: providers grant headroom, queued units reserve it, and the
//! production gate holds when it runs out.

mod utils;

use ferrets_content::player_stats::PlayerStatId;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand, components::build::UnderConstructionComponent,
    player_stats::PlayerStats, simulation_id::SimulationId, spawn, supply,
};

//
// ─── Gate on training ───────────────────────────────────────────────────────
//

#[test]
fn train_blocked_without_headroom() {
    let mut app = utils::supply_app();
    let (lodge, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, 10);

    // Nothing provides supply, so the order is refused outright: nothing
    // queued, nothing paid.
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 0);
    assert_eq!(utils::train_queue_len(app.world(), lodge), 0);
    assert!(utils::order_queue_is_empty(app.world_mut(), lodge));
    assert_eq!(utils::gold(app.world()), 50);
}

#[test]
fn provider_grants_headroom_and_training_proceeds() {
    let mut app = utils::supply_app();
    let (_, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    utils::spawn_owned(&mut app, "camp", 20, 20, 0);
    utils::grant_gold(&mut app, 50);

    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(8));
    assert_eq!(supply::used(app.world(), 0), FixedU64::ZERO);

    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, 20);

    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);
    assert_eq!(utils::gold(app.world()), 40);
    // The queued unit's reservation carried over to the standing settler.
    assert_eq!(supply::used(app.world(), 0), FixedU64::ONE);
    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(8));
}

#[test]
fn queued_units_reserve_supply() {
    let mut app = utils::supply_app();
    let (lodge, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    utils::spawn_owned(&mut app, "camp", 20, 20, 0);
    app.world_mut().resource_mut::<PlayerStats>().set_base(
        0,
        PlayerStatId::MAX_SUPPLY,
        FixedU64::from_num(2),
    );
    utils::grant_gold(&mut app, 100);

    for _ in 0..3 {
        train_settler(&mut app, lodge_id);
    }
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Two entries fill the ceiling at queue time; the third order is dropped
    // while the first two still sit in the queue, unspawned.
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 0);
    assert_eq!(utils::train_queue_len(app.world(), lodge), 2);
    assert_eq!(utils::gold(app.world()), 80);
    assert_eq!(supply::used(app.world(), 0), FixedU64::from_num(2));
    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(2));

    // The reserved pair finishes; the dropped third never appears.
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 2);
    assert_eq!(utils::gold(app.world()), 80);
}

//
// ─── Provider lifecycle ─────────────────────────────────────────────────────
//

#[test]
fn provider_death_blocks_new_training_only() {
    let mut app = utils::supply_app();
    let (_, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    let (camp, _) = utils::spawn_owned(&mut app, "camp", 20, 20, 0);
    utils::grant_gold(&mut app, 50);

    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);

    spawn::destroy_entity(app.world_mut(), camp);

    // The standing settler survives its provider, but the headroom is gone.
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);
    assert_eq!(supply::provided(app.world(), 0), FixedU64::ZERO);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ONE);

    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);
    assert_eq!(utils::gold(app.world()), 40);
}

#[test]
fn queued_unit_finishes_after_provider_dies() {
    let mut app = utils::supply_app();
    let (lodge, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    let (camp, _) = utils::spawn_owned(&mut app, "camp", 20, 20, 0);
    utils::grant_gold(&mut app, 50);

    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::train_queue_len(app.world(), lodge), 1);
    assert_eq!(utils::gold(app.world()), 40);

    spawn::destroy_entity(app.world_mut(), camp);

    // The reservation outlives the provider: the paid-for unit is never
    // stranded, it finishes and steps out.
    assert_eq!(supply::provided(app.world(), 0), FixedU64::ZERO);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ONE);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);
}

#[test]
fn under_construction_provider_feeds_nobody() {
    let mut app = utils::supply_app();
    let (_, pioneer_id) = utils::spawn_owned(&mut app, "pioneer", 9, 10, 0);
    utils::grant_gold(&mut app, 50);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: pioneer_id,
            type_name: "camp".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 3);

    // The site stands and is being raised, but provides nothing yet.
    assert_eq!(utils::count_of_type(app.world_mut(), "camp"), 1);
    let camp = utils::single_owned_of_type(app.world_mut(), "camp", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(camp)
            .is_some(),
        "camp is still going up"
    );
    assert_eq!(supply::provided(app.world(), 0), FixedU64::ZERO);

    // Completion turns the site into a provider.
    utils::run_ticks(&mut app, 20);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(camp)
            .is_none()
    );
    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(8));
}

//
// ─── Ceiling and exemptions ─────────────────────────────────────────────────
//

#[test]
fn max_supply_caps_provided() {
    let mut app = utils::supply_app();
    utils::spawn_owned(&mut app, "camp", 10, 10, 0);
    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(8));

    app.world_mut().resource_mut::<PlayerStats>().set_base(
        0,
        PlayerStatId::MAX_SUPPLY,
        FixedU64::from_num(3),
    );

    assert_eq!(supply::provided(app.world(), 0), FixedU64::from_num(3));
}

#[test]
fn zero_cost_type_trains_over_cap() {
    let mut app = utils::supply_app();
    let (_, lodge_id) = utils::spawn_owned(&mut app, "lodge", 10, 10, 0);
    // A settler placed directly puts the player over its (empty) supply.
    utils::spawn_owned(&mut app, "settler", 5, 5, 0);
    utils::grant_gold(&mut app, 50);
    assert_eq!(supply::provided(app.world(), 0), FixedU64::ZERO);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ONE);

    // A costless worker is admitted even over the cap...
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: lodge_id,
            type_name: "worker".into(),
        },
    );
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "worker"), 1);
    assert_eq!(utils::gold(app.world()), 40);

    // ...while a costed settler from the same trainer stays refused.
    train_settler(&mut app, lodge_id);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "settler"), 1);
    assert_eq!(utils::gold(app.world()), 40);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Orders `trainer` to train one settler.
fn train_settler(app: &mut bevy::prelude::App, trainer: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: "settler".into(),
        },
    );
}
