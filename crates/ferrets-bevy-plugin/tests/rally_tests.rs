//! Rally points: trained units take an order toward the producer's rally
//! target — a plain move for a position, a resolved send-to-entity intent for
//! an entity — and invalid rally commands are refused.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::{nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        location::Solidity,
        rally::{RallyPointComponent, RallyTarget},
        resource::ResourceSourceComponent,
    },
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    resources::PlayerResources,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

//
// ─── Rally consumption ──────────────────────────────────────────────────────
//

#[test]
fn trained_unit_moves_to_rally_position() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Position(utils::pos(5, 10))),
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );

    utils::run_ticks(&mut app, 30);
    let world = app.world_mut();
    let soldier = utils::single_owned_of_type(world, "soldier", 0);
    assert_eq!(utils::cell_of(world, soldier), NavPos::new(5, 10));
}

#[test]
fn rally_on_resource_source_makes_trained_worker_harvest() {
    let mut app = rally_app();
    let world = app.world_mut();
    let (_, hall_id) = spawn::spawn_entity(world, "hall", utils::pos(10, 10), Some(0)).unwrap();
    let (mine, mine_id) = spawn::spawn_entity(world, "mine", utils::pos(14, 10), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 100;
    spawn::spawn_entity(world, "depot", utils::pos(6, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 10);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: hall_id,
            target: Some(RallyTarget::Entity(mine_id)),
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: hall_id,
            type_name: "worker".into(),
        },
    );

    // Training spent the whole stockpile; anything above zero was harvested
    // from the rally target and delivered.
    utils::run_ticks(&mut app, 100);
    assert!(utils::gold(app.world_mut()) > 0);
}

#[test]
fn rally_on_hostile_target_makes_trained_soldier_attack() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    let (enemy, enemy_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(14, 10), Some(1)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Entity(enemy_id)),
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );

    utils::run_ticks(&mut app, 40);
    utils::assert_despawned(app.world_mut(), enemy);
}

#[test]
fn clearing_rally_point_leaves_trained_unit_at_spawn_cell() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Position(utils::pos(5, 10))),
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: None,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );

    utils::run_ticks(&mut app, 30);
    let world = app.world_mut();
    let soldier = utils::single_owned_of_type(world, "soldier", 0);
    utils::assert_adjacent_to_footprint(world, soldier, barracks);
    assert!(utils::order_queue_is_empty(world, soldier));
}

#[test]
fn rally_target_gone_before_spawn_leaves_trained_unit_without_orders() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    let (enemy, enemy_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(14, 10), Some(1)).unwrap();
    world.resource_mut::<PlayerResources>().add(0, "gold", 30);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Entity(enemy_id)),
        },
    );
    utils::run_ticks(&mut app, 1);

    // The rally target dies before production starts; the trained unit gets
    // nothing to head to and stays where it spawned.
    spawn::destroy_entity(app.world_mut(), enemy);
    utils::run_ticks(&mut app, 4);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: barracks_id,
            type_name: "soldier".into(),
        },
    );

    utils::run_ticks(&mut app, 15);
    let world = app.world_mut();
    let soldier = utils::single_owned_of_type(world, "soldier", 0);
    utils::assert_adjacent_to_footprint(world, soldier, barracks);
    assert!(utils::order_queue_is_empty(world, soldier));
}

//
// ─── Command validation ─────────────────────────────────────────────────────
//

#[test]
fn rally_on_foreign_trainer_is_refused() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(1)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Position(utils::pos(5, 10))),
        },
    );

    utils::run_ticks(&mut app, 2);
    let rally = app
        .world_mut()
        .get::<RallyPointComponent>(barracks)
        .unwrap();
    assert_eq!(rally.0, None);
}

#[test]
fn rally_on_entity_without_rally_capability_is_refused() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (soldier, soldier_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: soldier_id,
            target: Some(RallyTarget::Position(utils::pos(1, 1))),
        },
    );

    // A non-producer never carries rally state; the command is ignored.
    utils::run_ticks(&mut app, 2);
    assert!(
        app.world_mut()
            .get::<RallyPointComponent>(soldier)
            .is_none()
    );
}

#[test]
fn rally_on_vanished_target_is_refused() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (barracks, barracks_id) =
        spawn::spawn_entity(world, "barracks", utils::pos(10, 10), Some(0)).unwrap();
    let (enemy, enemy_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(14, 10), Some(1)).unwrap();

    spawn::destroy_entity(app.world_mut(), enemy);
    utils::run_ticks(&mut app, 4);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: barracks_id,
            target: Some(RallyTarget::Entity(enemy_id)),
        },
    );

    utils::run_ticks(&mut app, 2);
    let rally = app
        .world_mut()
        .get::<RallyPointComponent>(barracks)
        .unwrap();
    assert_eq!(rally.0, None);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// [`utils::orders_app`] extended with a `hall` producer that trains workers.
fn rally_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("hall")
                .with_location(utils::GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_trainer(["worker"]),
        );
    }
    utils::register_orders_content(&mut app);
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
