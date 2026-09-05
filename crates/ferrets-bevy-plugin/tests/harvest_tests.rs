//! Harvest order: carriers collecting from sources and delivering their loads.

mod utils;

use std::collections::BTreeSet;

use bevy::prelude::*;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        hidden::HiddenComponent,
        resource::{
            HarvestingComponent, ResourceCarrierComponent, ResourceSourceComponent,
            UnderHarvestComponent,
        },
    },
    entity_def,
    order::Order,
    resources::PlayerResources,
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn collect_harvests_until_source_depletes() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    // A source target resolves to a harvest order.
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );

    // Gold trips are hidden: the worker disappears into the mine while working.
    utils::run_ticks(&mut app, 10);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // 12 gold arrive over three trips (5 + 5 + 2), then the empty mine is removed
    // and the worker stops, back on the map.
    utils::run_ticks(&mut app, 63);
    assert_eq!(utils::gold(app.world_mut()), 12);
    utils::run_ticks(&mut app, 1);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    utils::run_ticks(&mut app, 1);
    utils::assert_despawned(app.world_mut(), mine);
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn visible_harvest_marks_carrier() {
    let mut app = utils::orders_app();
    let (lumberjack, lumberjack_id) = utils::create_owned(&mut app, "lumberjack", 5, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 4;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, lumberjack_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );

    // Wood trips are visible: the lumberjack stays on the map, marked as working.
    utils::run_ticks(&mut app, 10);
    assert!(
        app.world_mut()
            .get::<HarvestingComponent>(lumberjack)
            .is_some()
    );
    assert!(app.world_mut().get::<HiddenComponent>(lumberjack).is_none());

    // The load is delivered, the felled tree is removed, and the marker is gone.
    utils::run_ticks(&mut app, 13);
    assert_eq!(
        app.world_mut()
            .resource::<PlayerResources>()
            .amount(0, "wood"),
        4
    );
    utils::run_ticks(&mut app, 1);
    utils::assert_despawned(app.world_mut(), tree);
    let world = app.world_mut();
    assert!(world.get::<HarvestingComponent>(lumberjack).is_none());
}

#[test]
fn hidden_harvest_marks_carrier_inside_source() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );

    // Gold trips take the worker into the shaft. Off the map and at work are separate
    // facts, and a carrier down a mine is both.
    utils::run_ticks(&mut app, 10);
    let world = app.world_mut();
    assert!(world.get::<HiddenComponent>(worker).is_some());
    assert!(
        world.get::<HarvestingComponent>(worker).is_some(),
        "a carrier inside a source is working it"
    );
}

#[test]
fn source_emptied_mid_trip_lets_carrier_out() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (geyser, geyser_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 20;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: geyser_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 10);
    assert!(
        app.world().get::<HiddenComponent>(worker).is_some(),
        "the trip is under way, with the worker inside"
    );

    // The seam runs dry under a worker that is still inside it and carries nothing:
    // there is no load to deliver and nothing left to work, so the order ends — and
    // ending it has to put the worker back on the map.
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 0;
    utils::run_ticks(&mut app, 1);

    let world = app.world_mut();
    assert!(world.get::<HiddenComponent>(worker).is_none());
    assert!(world.get::<HarvestingComponent>(worker).is_none());
    assert!(world.get::<UnderHarvestComponent>(geyser).is_none());
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn persistent_source_stays_on_map_when_depleted() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (geyser, geyser_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 4;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: geyser_id,
            flush: true,
        },
    );

    // One trip empties the geyser; the load is delivered and the order finishes.
    utils::run_ticks(&mut app, 23);
    assert_eq!(utils::gold(app.world_mut()), 4);
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));

    // The empty geyser is still on the map.
    let world = app.world_mut();
    assert_eq!(
        world.get::<ResourceSourceComponent>(geyser).unwrap().amount,
        0
    );
}

#[test]
fn boxed_in_cancel_defers_reveal_until_cell_frees() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(7, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;

    // Gold trips are hidden: send the worker to disappear into the mine to work.
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 6);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Box the worker in — every cell is occupied, so it has nowhere to reappear —
    // then cancel the harvest while it is still inside the mine.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::stop_orders(app.world_mut(), worker);
    utils::run_ticks(&mut app, 1);

    // The cancel cannot retry itself, so rather than forcing an overlap it queues
    // the reveal; freeing the worker's own cell brings it back there.
    let anchor = utils::cell_of(app.world_mut(), worker);
    utils::assert_reveal_deferred_then_lands_on(&mut app, worker, anchor);
}

#[test]
fn harvest_range_reaches_source_but_not_drop_off() {
    let mut app = utils::orders_app();
    // Three cells short of the mine, which its harvest_range of 3 already covers.
    let (prospector, prospector_id) = utils::create_owned(&mut app, "prospector", 6, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 5;
    let (depot, _) = utils::create_owned(&mut app, "depot", 2, 4, 0);
    let stood_at = utils::cell_of(app.world_mut(), prospector);

    utils::select(&mut app, prospector_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );

    // The seam is worked from where it stands, so the trip starts the moment the
    // order lands rather than after a walk.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(
        app.world_mut()
            .get::<HarvestingComponent>(prospector)
            .is_some(),
        "the trip is under way"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), prospector),
        stood_at,
        "a longer reach means no step toward the mine at all"
    );

    // Delivering is another matter: the load has to be carried to the storage, not
    // lobbed at it from three cells back.
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::gold(app.world_mut()), 5);
    utils::assert_adjacent_to_footprint(app.world_mut(), prospector, depot);
}

//
// ─── Sharing a source ─────────────────────────────────────────────────────────
//

#[test]
fn crew_shares_one_source() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "logger", 8, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "logger", 10, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    send_both_to(&mut app, first_id, second_id, tree_id);

    // Both start in reach of the stand, and neither has to wait for the other.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world_mut().get::<HarvestingComponent>(first).is_some());
    assert!(app.world_mut().get::<HarvestingComponent>(second).is_some());
}

#[test]
fn worked_source_records_crew_until_last_carrier_leaves() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "logger", 8, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "logger", 10, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    assert!(
        crew_of(&app, tree).is_none(),
        "an untouched stand carries no crew"
    );

    send_both_to(&mut app, first_id, second_id, tree_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        crew_of(&app, tree),
        Some(BTreeSet::from([first_id, second_id])),
        "both carriers show up in the stand's crew"
    );

    // One of the pair stops. The other is still at work, so the stand keeps its crew.
    utils::stop_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        crew_of(&app, tree),
        Some(BTreeSet::from([second_id])),
        "one carrier leaving does not idle the stand"
    );

    // With nobody left on it the mark goes with the last carrier out.
    utils::stop_orders(app.world_mut(), second);
    utils::run_ticks(&mut app, 1);
    assert!(crew_of(&app, tree).is_none());
}

#[test]
fn carrier_that_works_alone_waits_for_source_another_holds() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "lumberjack", 8, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "lumberjack", 10, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    send_both_to(&mut app, first_id, second_id, tree_id);
    utils::run_ticks(&mut app, utils::APPLY);

    let world = app.world_mut();
    let working = [first, second]
        .into_iter()
        .filter(|carrier| world.get::<HarvestingComponent>(*carrier).is_some())
        .count();
    assert_eq!(working, 1, "one carrier has the stand and the other waits");
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), first)
            && !utils::order_queue_is_empty(app.world_mut(), second),
        "the one kept waiting holds its order rather than giving up"
    );
}

//
// ─── Reach from a body part way across its cells ───────────────────────────────
//

/// A carrier diagonally past a storage's corner is standing beside it and hands
/// its load over from there. Judging such a body by one of the two cells it lies
/// across reads it as out of range on whichever axis that cell rounds away, so
/// it would owe a step it is already done with.
#[test]
fn carrier_part_way_across_cells_delivers_from_corner_it_stands_on() {
    let mut app = utils::continuous_orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 12, 8, 0);
    let (_, depot_id) = utils::create_owned(&mut app, "depot", 10, 12, 0);

    // Walked to the spot rather than placed on it: a point move lands on the
    // ordered position to the bit, and only a walk leaves a body off the
    // lattice with its claims in order.
    let corner = utils::part_way("12.6", "10.6");
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: corner,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::position_of(app.world_mut(), worker), corner);
    load_with_gold(&mut app, worker);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: depot_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 3);

    assert_eq!(utils::gold(app.world()), 5, "the load went into the depot");
    assert_eq!(
        utils::position_of(app.world_mut(), worker),
        corner,
        "already in reach, so nothing was walked"
    );
}

//
// ─── Unreachable storage ──────────────────────────────────────────────────────
//

/// A load that cannot be walked anywhere is not a load the carrier stops
/// carrying: the trip waits for the way to open, as a blocked source does.
#[test]
fn carrier_waits_in_place_when_way_to_storage_is_shut() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (_, depot_id) = utils::create_owned(&mut app, "depot", 20, 20, 0);
    wall_in(&mut app, 20, 20, 2);
    load_with_gold(&mut app, worker);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: depot_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 300);

    assert_eq!(utils::gold(app.world()), 0, "the box still stands");
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), worker),
        "the only storage around is merely blocked, so the trip waits it out"
    );
    assert_eq!(
        app.world()
            .get::<ResourceCarrierComponent>(worker)
            .unwrap()
            .amount,
        5,
        "and the load is still in hand"
    );
}

#[test]
fn waiting_carrier_delivers_when_way_to_storage_opens() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (_, depot_id) = utils::create_owned(&mut app, "depot", 20, 20, 0);
    let walls = wall_in(&mut app, 20, 20, 2);
    load_with_gold(&mut app, worker);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: depot_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 100);
    assert_eq!(utils::gold(app.world()), 0, "the box still stands");

    for boulder in walls {
        spawn::destroy_entity(app.world_mut(), boulder);
    }
    utils::run_ticks(&mut app, 300);

    assert_eq!(
        utils::gold(app.world()),
        5,
        "the way opened and the waiting load went in"
    );
}

//
// ─── Unreachable sources and kind lock ────────────────────────────────────────
//

#[test]
fn carrier_switches_to_nearby_source_when_ordered_one_unreachable() {
    let mut app = utils::orders_app();
    let (_, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    // The ordered mine sits inside a boulder ring; another gold source stands
    // in the open beside it.
    let (walled, walled_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(20, 20), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(walled)
        .unwrap()
        .amount = 10;
    wall_in(&mut app, 20, 20, 1);
    let (open, _) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(16, 20), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(open)
        .unwrap()
        .amount = 5;

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: walled_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 400);

    assert_eq!(
        utils::gold(app.world()),
        5,
        "the load came from the open mine beside the walled one"
    );
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(walled)
            .unwrap()
            .amount,
        10,
        "the walled mine was never touched"
    );
}

#[test]
fn carrier_waits_in_place_when_unreachable_source_is_only_one_around() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    let (walled, walled_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(20, 20), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(walled)
        .unwrap()
        .amount = 10;
    wall_in(&mut app, 20, 20, 1);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: walled_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 300);

    assert!(
        !utils::order_queue_is_empty(app.world_mut(), worker),
        "the only source around is merely blocked, so the order waits it out"
    );
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn waiting_carrier_resumes_when_way_to_source_opens() {
    let mut app = utils::orders_app();
    let (_, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    let (walled, walled_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(20, 20), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(walled)
        .unwrap()
        .amount = 5;
    let ring = wall_in(&mut app, 20, 20, 1);

    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: walled_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 100);
    assert_eq!(utils::gold(app.world()), 0, "the ring still stands");

    for boulder in ring {
        spawn::destroy_entity(app.world_mut(), boulder);
    }
    utils::run_ticks(&mut app, 300);

    assert_eq!(
        utils::gold(app.world()),
        5,
        "the retry finds the way open and the trip completes"
    );
}

#[test]
fn foreign_load_is_wasted_at_first_transfer_not_delivered() {
    let mut app = utils::orders_app();
    let (forager, forager_id) = utils::create_owned(&mut app, "forager", 5, 5, 0);
    utils::create_owned(&mut app, "depot", 2, 4, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 5;

    // Three gold in hand when the order names wood: the worker walks straight
    // to the tree — no storage detour — and the gold is gone the moment the
    // wood is in hand.
    {
        let mut carrier = app
            .world_mut()
            .get_mut::<ResourceCarrierComponent>(forager)
            .unwrap();
        carrier.kind = Some("gold".to_string());
        carrier.amount = 3;
    }

    utils::select(&mut app, forager_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 200);

    assert_eq!(utils::gold(app.world()), 0, "the gold load was wasted");
    assert_eq!(utils::wood(app.world()), 5, "a clean wood load arrived");
    assert!(utils::order_queue_is_empty(app.world_mut(), forager));
}

#[test]
fn order_locked_to_wood_does_not_switch_to_gold() {
    let mut app = utils::orders_app();
    // The forager can carry either kind; only the order's lock is on trial.
    let (forager, forager_id) = utils::create_owned(&mut app, "forager", 5, 5, 0);
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    // One tree holding exactly one load, with a gold mine right beside it.
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 5;
    let (mine, _) = utils::create_entity(app.world_mut(), "mine", utils::pos(10, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 10;

    utils::select(&mut app, forager_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 200);

    assert_eq!(utils::wood(app.world()), 5, "the tree's one load arrived");
    assert_eq!(
        utils::gold(app.world()),
        0,
        "a wood order never drifts to gold, however close the mine stands"
    );
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(mine)
            .unwrap()
            .amount,
        10
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), forager),
        "with no wood left anywhere near, the order gives up"
    );
}

//
// ─── Whose storage takes a delivery ───────────────────────────────────────────
//

#[test]
fn loaded_carrier_sent_to_rival_storage_follows_it_instead_of_delivering() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    let (_, rival_depot) = utils::create_owned(&mut app, "depot", 14, 14, 1);
    load_with_gold(&mut app, worker);
    utils::run_ticks(&mut app, 1);

    utils::send_to(&mut app, worker_id, rival_depot);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let orders = entity_def::orders(app.world(), worker);
    assert!(
        orders
            .iter()
            .any(|order| matches!(order, Order::Follow { .. })),
        "a rival's storage takes nothing in, so the click falls through to following it"
    );
    assert!(
        !orders
            .iter()
            .any(|order| matches!(order, Order::Harvest { .. })),
        "and no delivery is queued"
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Fills `carrier`'s hands with a full load of gold, so the next thing its trip
/// does is look for somewhere to put it.
fn load_with_gold(app: &mut App, carrier: Entity) {
    let mut component = app
        .world_mut()
        .get_mut::<ResourceCarrierComponent>(carrier)
        .unwrap();
    component.kind = Some("gold".to_string());
    component.amount = 5;
}

/// Walls the `size`-by-`size` footprint at `(x, y)` in on every side with
/// boulders, so nothing outside can reach it. Returns them, for a test that
/// opens the way again.
fn wall_in(app: &mut App, x: u32, y: u32, size: u32) -> Vec<Entity> {
    let mut walls = Vec::new();
    let span = 0..size as i32;
    for dx in -1..=size as i32 {
        for dy in -1..=size as i32 {
            if span.contains(&dx) && span.contains(&dy) {
                continue;
            }
            let (bx, by) = ((x as i32 + dx) as u32, (y as i32 + dy) as u32);
            let (boulder, _) =
                utils::create_entity(app.world_mut(), "boulder", utils::pos(bx, by), None).unwrap();
            walls.push(boulder);
        }
    }
    walls
}

/// Selects both carriers and sends the pair to one target, as a player crowding a
/// source would.
fn send_both_to(app: &mut App, first: SimulationId, second: SimulationId, target: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::SelectById {
            id: first,
            mode: SelectMode::Replace,
        },
    );
    utils::push_command(
        app,
        PlayerCommand::SelectById {
            id: second,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(
        app,
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
}

/// The crew a source records, if any is working it.
fn crew_of(app: &App, source: Entity) -> Option<BTreeSet<SimulationId>> {
    app.world()
        .get::<UnderHarvestComponent>(source)
        .map(|crew| crew.carriers.clone())
}
