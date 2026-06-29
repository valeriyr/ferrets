//! Harvest order: collecting from sources, delivering loads, depletion policies,
//! and a boxed-in cancel that defers the carrier's reveal.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        hidden::HiddenComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        pending_reveal::PendingRevealComponent,
        resource::{HarvestingComponent, ResourceSourceComponent},
    },
    map::Map,
    resources::PlayerResources,
    spawn,
};

#[test]
fn collect_harvests_until_source_depletes() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    let (mine, mine_id) = spawn::spawn_entity(world, "mine", utils::pos(9, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;
    spawn::spawn_entity(world, "depot", utils::pos(2, 4), Some(0)).unwrap();

    // A source target resolves to a harvest order.
    utils::push_command(&mut app, PlayerCommand::SelectById { id: worker_id });
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
    let world = app.world_mut();
    let (lumberjack, lumberjack_id) =
        spawn::spawn_entity(world, "lumberjack", utils::pos(5, 5), Some(0)).unwrap();
    let (tree, tree_id) = spawn::spawn_entity(world, "tree", utils::pos(9, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(tree)
        .unwrap()
        .amount = 4;
    spawn::spawn_entity(world, "depot", utils::pos(2, 4), Some(0)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: lumberjack_id });
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
fn persistent_source_stays_on_map_when_depleted() {
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    let (geyser, geyser_id) = spawn::spawn_entity(world, "geyser", utils::pos(9, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(geyser)
        .unwrap()
        .amount = 4;
    spawn::spawn_entity(world, "depot", utils::pos(2, 4), Some(0)).unwrap();

    utils::push_command(&mut app, PlayerCommand::SelectById { id: worker_id });
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
    let world = app.world_mut();
    let (worker, worker_id) =
        spawn::spawn_entity(world, "worker", utils::pos(5, 5), Some(0)).unwrap();
    let (mine, mine_id) = spawn::spawn_entity(world, "mine", utils::pos(7, 5), None).unwrap();
    world
        .get_mut::<ResourceSourceComponent>(mine)
        .unwrap()
        .amount = 12;

    // Gold trips are hidden: send the worker to disappear into the mine to work.
    utils::push_command(&mut app, PlayerCommand::SelectById { id: worker_id });
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
    set_all_cells_occupied(app.world_mut(), true);
    app.world_mut()
        .get_mut::<OrderQueueComponent>(worker)
        .unwrap()
        .cancel_all(CancelPolicy::Force);

    // The cancel cannot retry itself, so rather than forcing an overlap it leaves
    // the worker hidden and queues the reveal.
    utils::run_ticks(&mut app, 1);
    assert!(
        app.world_mut()
            .get::<PendingRevealComponent>(worker)
            .is_some()
    );
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Free the worker's own cell; the scheduled retry brings it back on a later
    // tick and drops the marker.
    let anchor = utils::cell_of(app.world_mut(), worker);
    app.world_mut()
        .resource_mut::<Map>()
        .nav_grid_mut()
        .set_occupied(utils::GROUND, anchor, false);

    utils::run_ticks(&mut app, 1);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    assert!(
        app.world_mut()
            .get::<PendingRevealComponent>(worker)
            .is_none()
    );
    assert_eq!(utils::cell_of(app.world_mut(), worker), anchor);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Marks or clears every cell of the map's ground layer, used to box entities in.
fn set_all_cells_occupied(world: &mut World, occupied: bool) {
    let mut map = world.resource_mut::<Map>();
    let grid = map.nav_grid_mut();
    let (width, height) = (grid.width(), grid.height());
    for y in 0..height {
        for x in 0..width {
            grid.set_occupied(utils::GROUND, NavPos::new(x, y), occupied);
        }
    }
}
