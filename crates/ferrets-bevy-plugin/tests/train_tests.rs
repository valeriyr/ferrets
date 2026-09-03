//! Train order: spawning units, deducting cost, and rejecting invalid orders.

mod utils;

use ferrets_simulation::{
    command::PlayerCommand, components::build::UnderConstructionComponent,
    resources::PlayerResources, spawn,
};

#[test]
fn train_spawns_units_and_deducts_cost() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
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
        spawn::create_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world
        .entity_mut(barracks)
        .insert(UnderConstructionComponent::default());
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
