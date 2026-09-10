//! Harvest order: carriers collecting from sources and delivering their loads.

mod utils;

use std::collections::BTreeSet;

use bevy::prelude::*;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_physics::body;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        attached::AttachedComponent,
        hidden::HiddenComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        resource::{
            HarvestingComponent, ResourceCarrierComponent, ResourceSourceComponent,
            UnderHarvestComponent,
        },
    },
    entity_def,
    entity_index::EntityIndex,
    map::Map,
    order::{AttackTarget, Order},
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
// ─── Banking where the carrier stands ─────────────────────────────────────────
//

#[test]
fn attached_carrier_sits_in_tree_and_banks_wood_without_felling_it() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    // No storage anywhere: nothing is ever walked back.

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    // The order lands on the third tick and the sylph, already in reach, takes
    // the tree up: on the tree's own cell, at work, on the map.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());
    assert!(app.world().get::<HiddenComponent>(sylph).is_none());
    assert!(app.world().get::<HarvestingComponent>(sylph).is_some());
    assert_eq!(utils::cell_of(app.world(), sylph), CellPos::new(8, 5));
    let grid = app.world().resource::<Map>().nav_grid();
    assert!(
        !grid.is_claimed_by(utils::GROUND, CellPos::new(7, 5)),
        "the cell it stood on is free"
    );

    // Two ticks a load, five wood a load, straight into the stockpile — and
    // the tree loses nothing, so the carrier never leaves it.
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::wood(app.world()), 5);
    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::wood(app.world()), 20);
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(tree)
            .unwrap()
            .amount,
        20
    );
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());
    assert_eq!(
        app.world()
            .get::<ResourceCarrierComponent>(sylph)
            .unwrap()
            .amount,
        0,
        "nothing is ever in its hands"
    );
}

#[test]
fn attached_carriers_fill_berths_of_one_vein_and_drain_it_together() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "sylph", 7, 6, 0);
    let (vein, vein_id) =
        utils::create_entity(app.world_mut(), "vein", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .unwrap()
        .amount = 12;

    send_both_to(&mut app, first_id, second_id, vein_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        crew_of(&app, vein),
        Some(BTreeSet::from([first_id, second_id])),
        "both sit on the vein at once"
    );
    // Each with its middle on the rim berth nearest to where it stood, the
    // two west points (8.5, 5.5) and (8.5, 6.5) — positions name the corner,
    // half a cell up and left of the middle.
    assert_eq!(utils::position_of(app.world(), first), point("8.0", "5.0"));
    assert_eq!(utils::position_of(app.world(), second), point("8.0", "6.0"));

    // First load, two ticks in: 5 + 5 banked, 2 left in the vein. Second
    // load: the lower id takes the 2, the other finds nothing and stops.
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::gold(app.world()), 10);
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(vein)
            .unwrap()
            .amount,
        2
    );
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::gold(app.world()), 12);

    // The empty vein goes, and both carriers step back onto the grid beside
    // where it stood with nothing left to do.
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), vein);
    for carrier in [first, second] {
        assert!(app.world().get::<AttachedComponent>(carrier).is_none());
        assert!(utils::order_queue_is_empty(app.world_mut(), carrier));
    }
    assert_ne!(
        utils::cell_of(app.world(), first),
        utils::cell_of(app.world(), second),
        "each takes a cell of its own"
    );
}

#[test]
fn attached_carrier_waits_for_free_berth() {
    // Two berths on the vein, three carriers: the third stands by until a
    // berth frees, then takes it.
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (_, second_id) = utils::create_owned(&mut app, "sylph", 7, 6, 0);
    let (third, third_id) = utils::create_owned(&mut app, "sylph", 7, 7, 0);
    let (vein, vein_id) =
        utils::create_entity(app.world_mut(), "vein", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .unwrap()
        .amount = 1000;

    send_both_to(&mut app, first_id, second_id, vein_id);
    utils::select(&mut app, third_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: vein_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(app.world().get::<AttachedComponent>(third).is_none());
    assert!(
        app.world().get::<HarvestingComponent>(third).is_none(),
        "the third waits in place, not at work"
    );
    assert!(!utils::order_queue_is_empty(app.world_mut(), third));
    assert_eq!(
        crew_of(&app, vein),
        Some(BTreeSet::from([first_id, second_id]))
    );

    // The first is called off; its berth is the third's on the next tick.
    utils::stop_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 2);
    assert!(app.world().get::<AttachedComponent>(third).is_some());
    assert_eq!(utils::cell_of(app.world(), third), CellPos::new(8, 5));
    assert_eq!(
        crew_of(&app, vein),
        Some(BTreeSet::from([second_id, third_id]))
    );
}

#[test]
fn berth_group_seats_no_more_than_its_slots() {
    // The lode has three spots but seats two: the third carrier waits though
    // a spot stands free.
    let mut app = utils::orders_app();
    let (_, first_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (_, second_id) = utils::create_owned(&mut app, "sylph", 7, 6, 0);
    let (third, third_id) = utils::create_owned(&mut app, "sylph", 7, 7, 0);
    let (lode, lode_id) =
        utils::create_entity(app.world_mut(), "lode", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(lode)
        .unwrap()
        .amount = 1000;

    send_both_to(&mut app, first_id, second_id, lode_id);
    utils::select(&mut app, third_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: lode_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(
        crew_of(&app, lode),
        Some(BTreeSet::from([first_id, second_id]))
    );
    assert!(app.world().get::<AttachedComponent>(third).is_none());
    assert!(!utils::order_queue_is_empty(app.world_mut(), third));
}

#[test]
fn attached_carrier_turned_away_by_source_without_its_berths() {
    // The mine offers no berths at all, so a carrier that sits in a "rim"
    // has no way to work it: the order is refused before it starts. Pushed
    // straight onto the queue, so the refusal is the harvest's own and not a
    // smart send reading the mine some other way.
    let mut app = utils::orders_app();
    let (sylph, _) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 20;

    app.world_mut()
        .get_mut::<OrderQueueComponent>(sylph)
        .unwrap()
        .push(
            Order::Harvest { target: mine_id },
            Some(CancelPolicy::Force),
        );
    utils::run_ticks(&mut app, 2);
    assert!(utils::order_queue_is_empty(app.world_mut(), sylph));
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn roaming_carrier_crosses_straight_to_berth_of_its_own_pick() {
    let mut app = utils::orders_app();
    let (roamer, roamer_id) = utils::create_owned(&mut app, "roamer", 7, 5, 0);
    let (lode, lode_id) =
        utils::create_entity(app.world_mut(), "lode", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(lode)
        .unwrap()
        .amount = 1000;

    utils::select(&mut app, roamer_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: lode_id,
            flush: true,
        },
    );
    // Seated on the third tick with its middle on the nearest berth, (8.5, 5.5)
    // — a position names the footprint's corner, so it reads half a cell up
    // and left of that. Its stay starts one tick along — the first simulation
    // id's mix, 1364076727, is odd, against a two-tick dwell — so that same
    // tick its two ticks are up; of the two free berths its mix picks the
    // second, the south-east one at (9.5, 6.5), and it sets off straight
    // across the diagonal: one cell of ground under the isometric metric, a
    // quarter a tick, so a quarter of the way on the fourth tick and there on
    // the seventh.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::position_of(app.world(), roamer), point("8.0", "5.0"));
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::position_of(app.world(), roamer),
        point("8.25", "5.25")
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::position_of(app.world(), roamer), point("9.0", "6.0"));
    assert!(
        app.world().get::<HarvestingComponent>(roamer).is_some(),
        "moving between the berths never interrupts the work"
    );
}

#[test]
fn circling_carrier_walks_loop_to_next_berth() {
    let mut app = utils::orders_app();
    let (circler, circler_id) = utils::create_owned(&mut app, "circler", 7, 5, 0);
    let (lode, lode_id) =
        utils::create_entity(app.world_mut(), "lode", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(lode)
        .unwrap()
        .amount = 1000;

    utils::select(&mut app, circler_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: lode_id,
            flush: true,
        },
    );
    // Seated at (8.5, 5.5) on the third tick, its stay one tick along (the
    // first id's mix, 1364076727, is odd), so it sets off that tick — and the
    // odd mix also sends it backward round the loop, to the south-east spot
    // (9.5, 6.5) along the diagonal closing leg: one cell of ground under the
    // isometric metric, a quarter a tick, a quarter of the way on the fourth
    // tick and there on the seventh. Two full ticks there, and its second
    // hop's mix, 920564995, is odd too: backward again, one cell west to
    // (8.5, 6.5), a quarter of the way on the tenth tick, there on the
    // thirteenth.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        utils::position_of(app.world(), circler),
        point("8.0", "5.0")
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::position_of(app.world(), circler),
        point("8.25", "5.25")
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        utils::position_of(app.world(), circler),
        point("9.0", "6.0")
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        utils::position_of(app.world(), circler),
        point("8.75", "6.0")
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        utils::position_of(app.world(), circler),
        point("8.0", "6.0")
    );
}

#[test]
fn orbiting_carrier_circles_its_berth_without_leaving_it() {
    let mut app = utils::orders_app();
    let (hoverer, hoverer_id) = utils::create_owned(&mut app, "hoverer", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;

    utils::select(&mut app, hoverer_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    // Seated with its middle on the canopy, (8.5, 5.5), on the third tick —
    // read half a cell up and left, where the footprint's corner is. Its
    // circuit starts three quarter turns round — the first simulation id's
    // mix, 1364076727, is 3 past a multiple of four — and turns another
    // quarter that tick, so its middle is north of the berth by the
    // quarter-cell radius, then east, south and west on the ticks after, and
    // always rounds to the tree's cell.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        utils::position_of(app.world(), hoverer),
        point("8.0", "4.75")
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::position_of(app.world(), hoverer),
        point("8.25", "5.0")
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::position_of(app.world(), hoverer),
        point("8.0", "5.25")
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::position_of(app.world(), hoverer),
        point("7.75", "5.0")
    );
    assert_eq!(
        body::anchor(utils::position_of(app.world(), hoverer)),
        CellPos::new(8, 5)
    );
    // Two ticks a load, five wood a load, banked as it hovers: on the fourth
    // tick and the sixth.
    assert_eq!(utils::wood(app.world()), 10);
    assert!(app.world().get::<HarvestingComponent>(hoverer).is_some());
}

#[test]
fn circling_crew_spreads_round_loop() {
    // Two circlers sit down next to each other, at the north-west and
    // north-east spots nearest their approaches. Each, when it moves on,
    // skips the spot beside its neighbour for the one farthest from it.
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "circler", 7, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "circler", 10, 5, 0);
    let (ring, ring_id) =
        utils::create_entity(app.world_mut(), "ring", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(ring)
        .unwrap()
        .amount = 1000;

    send_both_to(&mut app, first_id, second_id, ring_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::position_of(app.world(), first), point("8.0", "5.0"));
    assert_eq!(utils::position_of(app.world(), second), point("9.0", "5.0"));

    // The first's stay is a tick along already and its mix is odd, so it
    // leaves that tick backward round the loop: the south-west spot, one leg
    // away and two steps from its neighbour, reached on the seventh tick. The
    // second's mix, 821347078, is even: it leaves on the fourth tick forward,
    // and of the free spots — the north-west the first vacated and the
    // south-east — both a step from the first's new spot, takes the nearer
    // ahead, the south-east, reached on the eighth.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::position_of(app.world(), first), point("8.0", "6.0"));
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::position_of(app.world(), second), point("9.0", "6.0"));
}

#[test]
fn roaming_carriers_never_share_berth() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "roamer", 7, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "roamer", 7, 6, 0);
    let (vein, vein_id) =
        utils::create_entity(app.world_mut(), "vein", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .unwrap()
        .amount = 1000;

    send_both_to(&mut app, first_id, second_id, vein_id);
    // Both berths taken, so neither ever finds a free one to cross to.
    for _ in 0..4 {
        utils::run_ticks(&mut app, 4);
        let (a, b) = (
            utils::cell_of(app.world(), first),
            utils::cell_of(app.world(), second),
        );
        assert_ne!(a, b, "two carriers in one berth");
        assert!([CellPos::new(8, 5), CellPos::new(8, 6)].contains(&a));
        assert!([CellPos::new(8, 5), CellPos::new(8, 6)].contains(&b));
    }
}

#[test]
fn attached_carrier_can_be_killed_at_work() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    let (_, soldier_id) = utils::create_owned(&mut app, "soldier", 9, 7, 1);

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());

    // The soldier is not the local player's, so its order is pushed straight
    // onto its queue: attack the sylph where it sits.
    let soldier = app
        .world()
        .resource::<EntityIndex>()
        .alive(soldier_id)
        .unwrap();
    app.world_mut()
        .get_mut::<OrderQueueComponent>(soldier)
        .unwrap()
        .push(
            Order::Attack {
                target: AttackTarget::Entity(sylph_id),
                leash: None,
            },
            Some(CancelPolicy::Force),
        );
    // Twenty ticks: the soldier's walk into range, then two hits of ten on a
    // twenty-health carrier.
    utils::run_ticks(&mut app, 20);
    utils::assert_despawned(app.world_mut(), sylph);
    assert_eq!(crew_of(&app, tree), None, "the tree is nobody's job now");
}

#[test]
fn attached_carrier_is_walked_through_and_never_pushed_under_continuous_model() {
    // Holding no cells means holding no body either: a walker crossing the
    // cell the carrier left, and the cell it sits on, meets nothing.
    let mut app = utils::continuous_orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 3, 5, 0);

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());
    let seat = utils::position_of(app.world(), sylph);

    // The soldier is sent onto the cell the sylph left, right beside the tree.
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(7, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::cell_of(app.world(), soldier), CellPos::new(7, 5));
    assert!(utils::order_queue_is_empty(app.world_mut(), soldier));
    assert_eq!(
        utils::position_of(app.world(), sylph),
        seat,
        "nothing shoulders an attached carrier aside"
    );
}

#[test]
fn stopped_attached_carrier_steps_back_beside_source() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());

    utils::stop_orders(app.world_mut(), sylph);
    utils::run_ticks(&mut app, 1);
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());
    assert!(app.world().get::<HarvestingComponent>(sylph).is_none());
    utils::assert_adjacent_to_footprint(app.world_mut(), sylph, tree);
    let cell = utils::cell_of(app.world(), sylph);
    let grid = app.world().resource::<Map>().nav_grid();
    assert!(grid.is_claimed_by(utils::GROUND, cell));
}

#[test]
fn direct_carrier_steps_back_when_source_runs_dry() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 5, 0);
    // A geyser holds its ground when emptied, so the drained source is still
    // standing there when the carrier looks for its next one.
    let (geyser, geyser_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 3;

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: geyser_id,
            flush: true,
        },
    );
    // Three ticks for the command, then two of drawing — the tick it sits down
    // is the first of them: the whole three arrive at once, since a drain of
    // five takes all there is.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::gold(app.world_mut()), 3);
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(geyser)
            .unwrap()
            .amount,
        0
    );

    // With nothing left to draw and no other seam near, the trip ends on the
    // next tick: the carrier is back on the grid and the geyser is nobody's job.
    utils::run_ticks(&mut app, 1);
    assert!(app.world().get::<AttachedComponent>(sylph).is_none());
    assert!(app.world().get::<HarvestingComponent>(sylph).is_none());
    assert_eq!(crew_of(&app, geyser), None);
    assert!(utils::order_queue_is_empty(app.world_mut(), sylph));
    // Back on the grid, holding a cell of its own again.
    let cell = utils::cell_of(app.world(), sylph);
    assert!(
        app.world()
            .resource::<Map>()
            .nav_grid()
            .is_claimed_by(utils::GROUND, cell)
    );
}

#[test]
fn direct_carrier_leaves_dry_source_behind_when_it_moves_on() {
    let mut app = utils::orders_app();
    let (sylph, sylph_id) = utils::create_owned(&mut app, "sylph", 8, 5, 0);
    // Two geysers: the near one runs dry and holds its ground, so the carrier
    // has to give it up before it can take the far one.
    let (near, near_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(near)
        .unwrap()
        .amount = 3;
    let (far, _) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(12, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(far)
        .unwrap()
        .amount = 10;

    utils::select(&mut app, sylph_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: near_id,
            flush: true,
        },
    );
    // Three ticks for the command, two to draw the near geyser dry, one to step
    // back onto the grid, one to take up the walk, and the rest of the nine for
    // the cells to the far geyser at half a cell a tick.
    utils::run_ticks(&mut app, utils::APPLY + 9);
    assert_eq!(
        crew_of(&app, near),
        None,
        "the emptied geyser is nobody's job once the carrier moves on"
    );
    assert_eq!(crew_of(&app, far), Some(BTreeSet::from([sylph_id])));
    assert!(app.world().get::<AttachedComponent>(sylph).is_some());
}

#[test]
fn drain_takes_less_from_source_than_carrier_banks() {
    let mut app = utils::orders_app();
    let (_, tapper_id) = utils::create_owned(&mut app, "tapper", 8, 5, 0);
    let (vein, vein_id) =
        utils::create_entity(app.world_mut(), "vein", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .unwrap()
        .amount = 10;

    utils::select(&mut app, tapper_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: vein_id,
            flush: true,
        },
    );
    // Every draw banks what the seam holds up to a load of five and takes two
    // out of it: from ten, that is 5 + 5 + 5 + 4 + 2 banked over five draws,
    // and the seam is empty.
    utils::run_ticks(&mut app, utils::APPLY + 12);
    assert_eq!(utils::gold(app.world_mut()), 21);
    assert_eq!(utils::count_of_type(app.world_mut(), "vein"), 0);
}

//
// ─── Berths given up ──────────────────────────────────────────────────────────
//

#[test]
fn carrier_killed_in_berth_gives_it_back() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    let (tree, tree_id) =
        utils::create_entity(app.world_mut(), "tree", utils::pos(8, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;

    utils::select(&mut app, first_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world().get::<AttachedComponent>(first).is_some());

    // Boxed in, so the berth is the only place it holds when it dies.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    spawn::destroy_entity(app.world_mut(), first);
    // Two ticks of dying, then the body is cleared on the next.
    utils::run_ticks(&mut app, 3);
    utils::assert_despawned(app.world_mut(), first);
    utils::set_all_cells_statically_occupied(app.world_mut(), false);

    // The canopy seats one: the seat the dead carrier held is free for the next.
    let (second, second_id) = utils::create_owned(&mut app, "sylph", 7, 5, 0);
    utils::select(&mut app, second_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: tree_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(app.world().get::<AttachedComponent>(second).is_some());
}

//
// ─── How many may work one source ─────────────────────────────────────────────
//

#[test]
fn hidden_crew_of_two_admits_two_and_leaves_third_waiting() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "paired_digger", 8, 5, 0);
    let (second, second_id) = utils::create_owned(&mut app, "paired_digger", 10, 5, 0);
    let (third, third_id) = utils::create_owned(&mut app, "paired_digger", 9, 6, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 60;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    send_both_to(&mut app, first_id, second_id, mine_id);
    utils::select(&mut app, third_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    // All three stand in reach of the seam, so the crew limit is the only
    // thing that can keep one out. Three ticks of a twenty-tick trip.
    utils::run_ticks(&mut app, utils::APPLY + 2);

    // The seam takes the two the carrier's crew limit allows, and both are
    // inside it while they work.
    assert_eq!(crew_of(&app, mine).map(|crew| crew.len()), Some(2));
    assert!(app.world().get::<HarvestingComponent>(first).is_some());
    assert!(app.world().get::<HarvestingComponent>(second).is_some());
    assert!(app.world().get::<HiddenComponent>(first).is_some());

    // The third is turned away and waits, rather than joining or giving up:
    // its order stands, so it takes the seam up when a place frees.
    assert!(app.world().get::<HarvestingComponent>(third).is_none());
    assert!(app.world().get::<HiddenComponent>(third).is_none());
    assert!(!utils::order_queue_is_empty(app.world_mut(), third));
}

#[test]
fn carrier_that_works_alone_shuts_out_one_that_would_share() {
    let mut app = utils::orders_app();
    let (lone, lone_id) = utils::create_owned(&mut app, "lone_digger", 8, 5, 0);
    let (paired, paired_id) = utils::create_owned(&mut app, "paired_digger", 10, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 40;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    // The one that works alone takes the seam first.
    send_to(&mut app, lone_id, mine_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(
        app.world().get::<HarvestingComponent>(lone).is_some(),
        "the first one down the shaft is working it"
    );

    // The second one's own terms would admit a crew of two, so only the crew
    // already on the seam can turn it away: the strictest worker on a job is
    // the one that decides, whichever of them asked first.
    send_to(&mut app, paired_id, mine_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(
        app.world().get::<HarvestingComponent>(paired).is_none(),
        "a crew of one that admits nobody turns away a carrier that would share"
    );
    assert_eq!(
        crew_of(&app, mine),
        Some(BTreeSet::from([lone_id])),
        "the seam is still worked by the one that claimed it"
    );
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), paired),
        "the one turned away holds its order rather than giving up"
    );
}

//
// ─── Which sources a kind will work ───────────────────────────────────────────
//

#[test]
fn carrier_turns_down_source_its_kind_does_not_list() {
    let mut app = utils::orders_app();
    let (tapper, tapper_id) = utils::create_owned(&mut app, "geyser_tapper", 8, 5, 0);
    let (mine, mine_id) =
        utils::create_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 60;
    utils::create_owned(&mut app, "depot", 2, 4, 0);

    utils::select(&mut app, tapper_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: mine_id,
            flush: true,
        },
    );
    // Standing in reach from the start, so a carrier the seam admitted would
    // be at work by the tick the order lands.
    utils::run_ticks(&mut app, utils::APPLY);

    // A bare seam is not a source it lists, so the gold in it is no business
    // of the tapper's.
    assert!(app.world().get::<HarvestingComponent>(tapper).is_none());
    assert!(crew_of(&app, mine).is_none());

    // The click is not swallowed, though: with nothing to work there it reads
    // as a plain approach, so the tapper holds an order rather than dropping
    // it, and stands where it already is.
    assert!(!utils::order_queue_is_empty(app.world_mut(), tapper));
    assert!(utils::within(app.world_mut(), tapper, mine, 1));

    // A geyser is, and the same order works it.
    let (geyser, geyser_id) =
        utils::create_entity(app.world_mut(), "geyser", utils::pos(7, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 60;
    utils::select(&mut app, tapper_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: geyser_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(app.world().get::<HarvestingComponent>(tapper).is_some());
    assert_eq!(crew_of(&app, geyser).map(|crew| crew.len()), Some(1));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A position from decimal strings, in cells.
fn point(x: &str, y: &str) -> FixedUVec2 {
    FixedUVec2::new(utils::fixed(x), utils::fixed(y))
}

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
/// Selects `carrier` alone and sends it to `target`.
fn send_to(app: &mut App, carrier: SimulationId, target: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::SelectById {
            id: carrier,
            mode: SelectMode::Replace,
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
