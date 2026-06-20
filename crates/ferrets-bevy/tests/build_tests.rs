//! Build order: constructing a building, and cancelling a build in progress.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        build::UnderConstructionComponent,
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
    },
    resources::PlayerResources,
    spawn,
};

#[test]
fn build_constructs_a_building() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );

    // The worker walks to the site, pays, hides inside, and the building appears
    // under construction.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    {
        let world = app.world_mut();
        assert_eq!(utils::gold(world), 30);
        assert!(world.get::<HiddenComponent>(worker).is_some());
        let under_construction = world
            .query_filtered::<&EntityInfoComponent, With<UnderConstructionComponent>>()
            .iter(world)
            .any(|info| info.type_name() == "depot");
        assert!(under_construction);
    }

    // Construction completes: the marker is gone and the worker reappears next to
    // the building.
    utils::run_ticks(&mut app, 6);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    let world = app.world_mut();
    let still_under_construction = world
        .query_filtered::<&EntityInfoComponent, With<UnderConstructionComponent>>()
        .iter(world)
        .count();
    assert_eq!(still_under_construction, 0);

    let worker_cell = utils::cell_of(world, worker);
    let nearest = NavPos::new(worker_cell.x.clamp(10, 11), worker_cell.y.clamp(10, 11));
    assert!(utils::chebyshev(worker_cell, nearest) <= 1);
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));

    // A constructible type outside the worker's catalogue is rejected.
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "barracks".into(),
            position: utils::pos(20, 20),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::count_of_type(app.world_mut(), "barracks"), 0);
    assert_eq!(utils::gold(app.world_mut()), 30);
}

#[test]
fn cancelling_a_build_refunds_and_restores_the_builder() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );

    // Wait until construction has started (cost paid, builder hidden inside).
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::gold(app.world_mut()), 30);

    // A hidden builder is not selectable, so a Stop command cannot reach it —
    // cancel the order directly, as a cancel-construction command would.
    app.world_mut()
        .get_mut::<OrderQueueComponent>(worker)
        .unwrap()
        .cancel_all(CancelPolicy::Force);

    // The cancel destroys the unfinished building, refunds the cost, and the
    // builder reappears next to the site.
    utils::run_ticks(&mut app, 1);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    assert_eq!(utils::gold(app.world_mut()), 80);
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}
