//! Train order: spawning units, deducting cost, and rejecting invalid orders.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{entity_info::EntityInfoComponent, location::LocationComponent},
    resources::PlayerResources,
    spawn,
};

#[test]
fn train_spawns_units_and_deducts_cost() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
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
    let origin = utils::cell_of(world, barracks);
    let positions: Vec<NavPos> = world
        .query::<(&EntityInfoComponent, &LocationComponent)>()
        .iter(world)
        .filter(|(info, _)| info.type_name() == "soldier")
        .map(|(_, loc)| NavPos::from(loc.position))
        .collect();
    for unit_cell in positions {
        let nearest = NavPos::new(
            unit_cell.x.clamp(origin.x, origin.x + 1),
            unit_cell.y.clamp(origin.y, origin.y + 1),
        );
        assert!(utils::chebyshev(unit_cell, nearest) <= 1);
    }
}
