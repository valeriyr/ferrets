//! Harvest order: carriers collecting from sources and delivering their loads.

mod utils;

use std::collections::BTreeSet;

use bevy::prelude::*;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        hidden::HiddenComponent,
        resource::{HarvestingComponent, ResourceSourceComponent, UnderHarvestComponent},
    },
    resources::PlayerResources,
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn collect_harvests_until_source_depletes() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        spawn::spawn_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (lumberjack, lumberjack_id) = utils::spawn_owned(&mut app, "lumberjack", 5, 5, 0);
    let (tree, tree_id) =
        spawn::spawn_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 4;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        spawn::spawn_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    let (geyser, geyser_id) =
        spawn::spawn_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 20;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    let (geyser, geyser_id) =
        spawn::spawn_entity(app.world_mut(), "geyser", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 4;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    let (mine, mine_id) =
        spawn::spawn_entity(app.world_mut(), "mine", utils::pos(7, 5), None).unwrap();
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
    utils::set_all_cells_occupied(app.world_mut(), true);
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
    let (prospector, prospector_id) = utils::spawn_owned(&mut app, "prospector", 6, 5, 0);
    let (mine, mine_id) =
        spawn::spawn_entity(app.world_mut(), "mine", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 5;
    let (depot, _) = utils::spawn_owned(&mut app, "depot", 2, 4, 0);
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
    let (first, first_id) = utils::spawn_owned(&mut app, "logger", 8, 5, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "logger", 10, 5, 0);
    let (tree, tree_id) =
        spawn::spawn_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

    send_both_to(&mut app, first_id, second_id, tree_id);

    // Both start in reach of the stand, and neither has to wait for the other.
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world_mut().get::<HarvestingComponent>(first).is_some());
    assert!(app.world_mut().get::<HarvestingComponent>(second).is_some());
}

#[test]
fn worked_source_records_crew_until_last_carrier_leaves() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::spawn_owned(&mut app, "logger", 8, 5, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "logger", 10, 5, 0);
    let (tree, tree_id) =
        spawn::spawn_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
    let (first, first_id) = utils::spawn_owned(&mut app, "lumberjack", 8, 5, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "lumberjack", 10, 5, 0);
    let (tree, tree_id) =
        spawn::spawn_entity(app.world_mut(), "tree", utils::pos(9, 5), None).unwrap();
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 20;
    utils::spawn_owned(&mut app, "depot", 2, 4, 0);

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
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

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
