//! SendToEntity command dispatch: resolving a target into the right order.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    resource::HarvestData,
    work::WorkPresence,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::resource::{ResourceCarrierComponent, ResourceSourceComponent},
    resources::PlayerResources,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
};

//
// ─── Hostiles and fallbacks ───────────────────────────────────────────────────
//

#[test]
fn send_to_entity_attacks_hostiles() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, attacker_id) =
        utils::create_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (enemy, enemy_id) =
        utils::create_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();

    utils::select(&mut app, attacker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: enemy_id,
            flush: true,
        },
    );

    // The dispatch resolved to an attack: the enemy is chased down and killed.
    utils::run_ticks(&mut app, 18);
    utils::assert_despawned(app.world_mut(), enemy);
}

#[test]
fn send_to_entity_falls_through_to_follow_for_uncarryable_kinds() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        utils::create_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    let (tree, tree_id) = utils::create_entity(world, "tree", utils::pos(10, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 10;

    // The worker carries only gold, so a wood source resolves to a follow.
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );

    utils::run_ticks(&mut app, 11);
    {
        let world = app.world_mut();
        assert!(utils::within(world, worker, tree, 1));
    }

    // Nothing was harvested: the stockpile and the source are untouched.
    let world = app.world_mut();
    assert_eq!(world.resource::<PlayerResources>().amount(0, "wood"), 0);
    assert_eq!(
        world.get::<ResourceSourceComponent>(tree).unwrap().amount,
        10
    );
}

//
// ─── Mending a friendly ───────────────────────────────────────────────────────
//

#[test]
fn send_to_entity_resolves_repair_for_damaged_friendly() {
    let mut app = repair_dispatch_app();
    let (warehouse, warehouse_id) = utils::create_owned(&mut app, "warehouse", 10, 10, 0);
    let (_, handyman_id) = utils::create_owned(&mut app, "handyman", 9, 10, 0);
    utils::wound(&mut app, warehouse, "20");

    utils::select(&mut app, handyman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: warehouse_id,
            flush: true,
        },
    );

    // The dispatch resolved to a repair: the handyman mends five points a tick, and
    // nothing else in the world heals.
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(
        utils::current_health(&app, warehouse),
        FixedU64::from_num(100),
        "the damage was mended off the same click that would harvest or attack"
    );
}

#[test]
fn loaded_carrier_sent_to_damaged_storage_still_delivers() {
    let mut app = repair_dispatch_app();
    let (warehouse, warehouse_id) = utils::create_owned(&mut app, "warehouse", 10, 10, 0);
    let (handyman, handyman_id) = utils::create_owned(&mut app, "handyman", 9, 10, 0);
    utils::wound(&mut app, warehouse, "20");
    // A full load, as if the handyman had just walked out of a mine.
    {
        let mut carrier = app
            .world_mut()
            .get_mut::<ResourceCarrierComponent>(handyman)
            .unwrap();
        carrier.kind = Some("gold".into());
        carrier.amount = 5;
    }

    utils::select(&mut app, handyman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: warehouse_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);

    assert_eq!(
        utils::gold(app.world()),
        5,
        "the load went into the stockpile"
    );
    assert_eq!(
        utils::current_health(&app, warehouse),
        FixedU64::from_num(80),
        "delivery outranks mending: the same click started no repair"
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// One human player and the pair the repair-dispatch tests need: a `warehouse` that
/// stores gold, and a `handyman` that both carries gold and mends buildings.
fn repair_dispatch_app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("warehouse")
                .with_location(utils::GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_tags(["building"])
                .with_resource_storage(["gold"]),
        );
        registry.register(
            EntityTypeDef::new("handyman")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Present))])
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_repairer(
                    ["building"],
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present,
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
