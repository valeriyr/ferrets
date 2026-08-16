//! Transports and garrisons: boarding, metered loading and unloading, holder
//! death fates, and passengers fighting from inside.

mod utils;

use ferrets_content::{entity_stats::EntityStatId, stats::ModifierOp};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        hidden::HiddenComponent,
        order_queue::OrderQueueComponent,
        pending_reveal::PendingRevealComponent,
        rally::RallyTarget,
        transport::{BoardedComponent, GarrisonFireComponent},
    },
    entity_index::EntityIndex,
    game_loop,
    map::Map,
    spawn,
};
use utils::{
    APPLY, GROUND, cell_of, health, passengers_of, pos, push_command, run_ticks, run_until_aboard,
    select, selection, send_to, set_all_cells_statically_occupied, single_owned_of_type,
    spawn_owned, transport_app, unload, within,
};

//
// ─── Boarding ───────────────────────────────────────────────────────────────
//

#[test]
fn unit_boards_transport_and_leaves_map_and_selection() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 14, 10, 0);

    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 40);

    assert_eq!(passengers_of(app.world(), wagon), [rifleman_id]);
    assert!(app.world().get::<HiddenComponent>(rifleman).is_some());
    assert_eq!(
        app.world()
            .get::<BoardedComponent>(rifleman)
            .map(|boarded| boarded.holder),
        Some(wagon_id)
    );
    assert!(
        !selection(&app).contains(&rifleman_id),
        "a unit off the map leaves the selection"
    );
}

#[test]
fn untransportable_unit_does_not_board() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (civilian, civilian_id) = spawn_owned(&mut app, "civilian", 13, 10, 0);

    send_to(&mut app, civilian_id, wagon_id);
    run_ticks(&mut app, 30);

    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(
        app.world().get::<HiddenComponent>(civilian).is_none(),
        "a unit without cargo_size stays in the open"
    );
}

#[test]
fn admission_list_matches_type_name() {
    let mut app = transport_app();
    // The bunker carries "rifleman" by type name; a grunt's "infantry" tag is
    // not on its list.
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (_, rifleman_id) = spawn_owned(&mut app, "rifleman", 13, 10, 0);
    let (grunt, grunt_id) = spawn_owned(&mut app, "grunt", 13, 12, 0);

    send_to(&mut app, rifleman_id, bunker_id);
    send_to(&mut app, grunt_id, bunker_id);
    run_ticks(&mut app, 30);

    assert_eq!(passengers_of(app.world(), bunker), [rifleman_id]);
    assert!(app.world().get::<HiddenComponent>(grunt).is_none());
}

#[test]
fn debuff_sealing_hold_turns_boarder_away() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);

    // Capacity has no fold floor, so a debuff can seal the hold entirely.
    let sealed = utils::register_entity_buff(
        &mut app,
        "sealed_hold",
        EntityStatId::CARGO_CAPACITY,
        ModifierOp::FlatAdd,
        -10.0,
        None,
    );
    game_loop::stats::apply_entity_buff(app.world_mut(), wagon, sealed);

    send_to(&mut app, rifleman_id, wagon_id);
    run_ticks(&mut app, 20);

    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(
        app.world().get::<HiddenComponent>(rifleman).is_none(),
        "a sealed hold admits nobody"
    );
}

#[test]
fn full_transport_turns_boarder_away() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    // Two grunts at cargo size 2 each fill the four slots.
    let (_, first_id) = spawn_owned(&mut app, "grunt", 12, 10, 0);
    let (_, second_id) = spawn_owned(&mut app, "grunt", 12, 11, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 12, 0);

    send_to(&mut app, first_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 40);
    send_to(&mut app, second_id, wagon_id);
    run_until_aboard(&mut app, wagon, 2, 40);

    send_to(&mut app, rifleman_id, wagon_id);
    run_ticks(&mut app, 30);

    assert_eq!(passengers_of(app.world(), wagon).len(), 2);
    assert!(
        app.world().get::<HiddenComponent>(rifleman).is_none(),
        "no slot left for even a one-slot passenger"
    );
}

#[test]
fn own_boarding_policy_rejects_allied_holder() {
    let mut app = transport_app();
    // Player 1 is allied with the local player 0; the wagon admits own only.
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 1);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 13, 10, 0);

    send_to(&mut app, rifleman_id, wagon_id);
    run_ticks(&mut app, 30);

    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(app.world().get::<HiddenComponent>(rifleman).is_none());
}

#[test]
fn allies_boarding_policy_admits_allied_unit() {
    let mut app = transport_app();
    let (ferry, ferry_id) = spawn_owned(&mut app, "ferry", 10, 10, 1);
    let (_, rifleman_id) = spawn_owned(&mut app, "rifleman", 13, 10, 0);

    send_to(&mut app, rifleman_id, ferry_id);
    run_until_aboard(&mut app, ferry, 1, 40);

    assert_eq!(passengers_of(app.world(), ferry), [rifleman_id]);
}

#[test]
fn load_period_meters_boardings() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    // Both stand inside the load range already, so they arrive together.
    let (_, first_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, second_id) = spawn_owned(&mut app, "rifleman", 11, 11, 0);

    select(&mut app, first_id);
    push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: second_id,
            mode: ferrets_simulation::command::SelectMode::Add,
        },
    );
    push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: wagon_id,
            flush: true,
        },
    );

    run_until_aboard(&mut app, wagon, 1, 10);
    assert_eq!(
        passengers_of(app.world(), wagon).len(),
        1,
        "the boarding cooldown holds the second arrival outside"
    );
    // The second boards once the three-tick loading period has passed.
    run_ticks(&mut app, 3);
    assert_eq!(passengers_of(app.world(), wagon).len(), 2);
}

#[test]
fn explicit_board_command_boards_mixed_selection() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    let (civilian, civilian_id) = spawn_owned(&mut app, "civilian", 12, 11, 0);

    select(&mut app, rifleman_id);
    push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: civilian_id,
            mode: SelectMode::Add,
        },
    );
    push_command(
        &mut app,
        PlayerCommand::Board {
            target: wagon_id,
            flush: true,
        },
    );
    run_until_aboard(&mut app, wagon, 1, 30);

    assert_eq!(
        passengers_of(app.world(), wagon),
        [rifleman_id],
        "the eligible unit boards"
    );
    assert!(app.world().get::<HiddenComponent>(rifleman).is_some());
    assert!(
        app.world().get::<HiddenComponent>(civilian).is_none(),
        "the ineligible unit drops the order and stays out"
    );
}

#[test]
fn explicit_follow_command_tails_own_transporter() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);

    // The smart click would read this pairing as boarding; the explicit
    // command keeps the rifleman outside, tailing the wagon.
    select(&mut app, rifleman_id);
    push_command(
        &mut app,
        PlayerCommand::Follow {
            target: wagon_id,
            flush: true,
        },
    );
    run_ticks(&mut app, 20);

    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(app.world().get::<HiddenComponent>(rifleman).is_none());
    assert!(
        within(app.world_mut(), rifleman, wagon, 2),
        "the follower stays close without climbing in"
    );
}

#[test]
fn transport_fetches_targeted_unit_aboard() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 5, 5, 0);
    let (grunt, grunt_id) = spawn_owned(&mut app, "grunt", 14, 5, 0);

    // The grunt is walking its own way; the wagon is sent to fetch it.
    select(&mut app, grunt_id);
    push_command(
        &mut app,
        PlayerCommand::Move {
            target: pos(14, 12),
            flush: true,
        },
    );
    push_command(
        &mut app,
        PlayerCommand::Load {
            transport: wagon_id,
            target: grunt_id,
            flush: true,
        },
    );
    run_until_aboard(&mut app, wagon, 1, 80);

    assert_eq!(passengers_of(app.world(), wagon), [grunt_id]);
    assert!(app.world().get::<HiddenComponent>(grunt).is_some());
    assert!(
        app.world()
            .get::<OrderQueueComponent>(grunt)
            .is_some_and(|queue| queue.front().is_none()),
        "the fetched unit's own orders are cancelled"
    );
}

#[test]
fn load_refuses_ineligible_unit() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 5, 5, 0);
    let (civilian, civilian_id) = spawn_owned(&mut app, "civilian", 8, 5, 0);

    push_command(
        &mut app,
        PlayerCommand::Load {
            transport: wagon_id,
            target: civilian_id,
            flush: true,
        },
    );
    run_ticks(&mut app, 20);

    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(
        app.world().get::<HiddenComponent>(civilian).is_none(),
        "a unit without cargo_size is not fetched"
    );
}

//
// ─── Unloading ──────────────────────────────────────────────────────────────
//

#[test]
fn unload_period_meters_exits_in_id_order() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (first, first_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    let (second, second_id) = spawn_owned(&mut app, "rifleman", 11, 11, 0);
    send_to(&mut app, first_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);
    send_to(&mut app, second_id, wagon_id);
    run_until_aboard(&mut app, wagon, 2, 10);

    unload(&mut app, wagon_id, None);
    run_ticks(&mut app, APPLY + 1);

    assert!(
        app.world().get::<HiddenComponent>(first).is_none(),
        "the lower id steps out first"
    );
    assert!(app.world().get::<HiddenComponent>(second).is_some());

    // The second follows after the two-tick unloading period.
    run_ticks(&mut app, 2);
    assert!(app.world().get::<HiddenComponent>(second).is_none());
    assert!(passengers_of(app.world(), wagon).is_empty());
    assert!(app.world().get::<BoardedComponent>(first).is_none());
}

#[test]
fn unmetered_holder_empties_in_one_tick() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (first, first_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    let (second, second_id) = spawn_owned(&mut app, "rifleman", 12, 11, 0);
    send_to(&mut app, first_id, bunker_id);
    send_to(&mut app, second_id, bunker_id);
    run_until_aboard(&mut app, bunker, 2, 40);

    unload(&mut app, bunker_id, None);
    run_ticks(&mut app, APPLY + 1);

    assert!(app.world().get::<HiddenComponent>(first).is_none());
    assert!(app.world().get::<HiddenComponent>(second).is_none());
    assert!(passengers_of(app.world(), bunker).is_empty());
}

#[test]
fn unload_at_point_walks_there_and_sends_passengers_on() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 5, 5, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 6, 5, 0);
    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);

    unload(&mut app, wagon_id, Some(pos(20, 5)));
    run_ticks(&mut app, 80);

    assert!(app.world().get::<HiddenComponent>(rifleman).is_none());
    let wagon_cell = cell_of(app.world_mut(), wagon);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(wagon_cell, CellPos::new(20, 5), 1),
        "the wagon walked into unload range of the point, stands at {wagon_cell:?}"
    );
    // The freed passenger marches on toward the point itself.
    let rifleman_cell = cell_of(app.world_mut(), rifleman);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(rifleman_cell, CellPos::new(20, 5), 1),
        "the passenger finished the trip on foot, stands at {rifleman_cell:?}"
    );
}

#[test]
fn immobile_holder_unloads_in_place_toward_point() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);

    unload(&mut app, bunker_id, Some(pos(20, 10)));
    run_ticks(&mut app, APPLY + 1);

    assert!(
        app.world().get::<HiddenComponent>(rifleman).is_none(),
        "a holder that cannot walk lets everyone out where it stands"
    );
    run_ticks(&mut app, 60);
    let rifleman_cell = cell_of(app.world_mut(), rifleman);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(rifleman_cell, CellPos::new(20, 10), 1),
        "the passenger still walks to the point, stands at {rifleman_cell:?}"
    );
}

#[test]
fn rally_point_sends_unloaded_passenger_on() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);

    push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: wagon_id,
            target: Some(RallyTarget::Position(pos(18, 10))),
        },
    );
    unload(&mut app, wagon_id, None);
    run_ticks(&mut app, 60);

    let rifleman_cell = cell_of(app.world_mut(), rifleman);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(rifleman_cell, CellPos::new(18, 10), 1),
        "the freed passenger walked to the rally point, stands at {rifleman_cell:?}"
    );
}

#[test]
fn blocked_exit_holds_passenger_and_retries() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);

    set_all_cells_statically_occupied(app.world_mut(), true);
    unload(&mut app, wagon_id, None);
    run_ticks(&mut app, APPLY + 4);

    assert!(
        app.world().get::<HiddenComponent>(rifleman).is_some(),
        "a boxed-in exit keeps the passenger aboard"
    );
    assert_eq!(passengers_of(app.world(), wagon), [rifleman_id]);

    // Free one cell beside the wagon; the held exit lands on it.
    app.world_mut()
        .resource_mut::<Map>()
        .set_static_occupied(GROUND, CellPos::new(11, 10), false);
    run_ticks(&mut app, 2);
    assert!(app.world().get::<HiddenComponent>(rifleman).is_none());
    assert!(passengers_of(app.world(), wagon).is_empty());
}

//
// ─── Holder death ───────────────────────────────────────────────────────────
//

#[test]
fn destroyed_wagon_takes_passengers_with_it() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);

    spawn::destroy_entity(app.world_mut(), wagon);
    run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(rifleman_id)
            .is_none(),
        "a destroy-fated holder kills its passengers"
    );
    utils::assert_despawned(app.world_mut(), rifleman);
}

#[test]
fn destroyed_bunker_ejects_passengers() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);

    spawn::destroy_entity(app.world_mut(), bunker);
    run_ticks(&mut app, 1);

    assert!(
        app.world().get::<HiddenComponent>(rifleman).is_none(),
        "an eject-fated holder puts its passengers back on the map"
    );
    assert!(app.world().get::<BoardedComponent>(rifleman).is_none());
    assert!(
        within(app.world_mut(), rifleman, bunker, 2),
        "ejected beside the holder's footprint"
    );
}

#[test]
fn ejected_passenger_without_room_dies() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);

    set_all_cells_statically_occupied(app.world_mut(), true);
    spawn::destroy_entity(app.world_mut(), bunker);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(rifleman_id)
            .is_none(),
        "a passenger the ring scan cannot place dies with its holder"
    );
    assert!(
        app.world()
            .get::<PendingRevealComponent>(rifleman)
            .is_none(),
        "nothing lingers hidden waiting for a holder that is gone"
    );
}

//
// ─── Garrison attack ────────────────────────────────────────────────────────
//

#[test]
fn garrisoned_rifleman_fires_from_bunker() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (_, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);

    // A hostile in weapon range of the bunker, and one beyond it.
    let (near, _) = spawn_owned(&mut app, "grunt", 13, 10, 2);
    let (far, _) = spawn_owned(&mut app, "grunt", 20, 10, 2);
    let near_health = health(&app, near);
    run_ticks(&mut app, 40);

    assert!(
        health(&app, near) < near_health,
        "the passenger's own weapon works the target from the holder"
    );
    assert_eq!(
        health(&app, far),
        20,
        "a target beyond the passenger's range is untouched"
    );
}

#[test]
fn garrison_fire_stops_once_unloaded() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);

    unload(&mut app, bunker_id, None);
    run_ticks(&mut app, APPLY + 1);
    assert!(app.world().get::<HiddenComponent>(rifleman).is_none());
    assert!(
        app.world().get::<GarrisonFireComponent>(rifleman).is_none(),
        "the firing state leaves with the passenger"
    );
}

#[test]
fn wagon_passengers_do_not_fire() {
    let mut app = transport_app();
    let (wagon, wagon_id) = spawn_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rifleman_id) = spawn_owned(&mut app, "rifleman", 11, 10, 0);
    send_to(&mut app, rifleman_id, wagon_id);
    run_until_aboard(&mut app, wagon, 1, 10);

    let (near, _) = spawn_owned(&mut app, "grunt", 13, 10, 2);
    run_ticks(&mut app, 40);

    assert_eq!(
        health(&app, near),
        20,
        "a sheltering holder keeps its cargo quiet"
    );
}

//
// ─── Hidden entities and blasts ─────────────────────────────────────────────
//

#[test]
fn blast_spares_hidden_passenger_at_stale_position() {
    let mut app = transport_app();
    let (bunker, bunker_id) = spawn_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = spawn_owned(&mut app, "rifleman", 12, 10, 0);
    send_to(&mut app, rifleman_id, bunker_id);
    run_until_aboard(&mut app, bunker, 1, 30);
    let full = health(&app, rifleman);

    // A hostile bombard auto-engages the bunker; its blast covers the cell the
    // rifleman stood on when it boarded.
    spawn_owned(&mut app, "bombard", 16, 10, 2);
    run_ticks(&mut app, 30);

    let bunker = single_owned_of_type(app.world_mut(), "bunker", 0);
    assert!(
        health(&app, bunker) < 200,
        "the shells land on the bunker itself"
    );
    assert_eq!(
        health(&app, rifleman),
        full,
        "a passenger is beyond any blast's reach"
    );
}
