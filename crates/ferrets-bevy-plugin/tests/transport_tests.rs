//! Transports and garrisons: boarding, metered loading and unloading, holder
//! death fates, and passengers fighting from inside.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    attack::Slain,
    entity_buffs::{EntityBuffDef, Lasting},
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    registry::ContentRegistry,
    stack_rule::StackRule,
    stats::ModifierOp,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_simulation::{
    beside::{Beside, Lean},
    command::{PlayerCommand, SelectMode},
    components::{
        hidden::{HiddenComponent, Inside},
        location::LocationComponent,
        order_queue::OrderQueueComponent,
        rally::RallyTarget,
        transport::{BoardedComponent, GarrisonFireComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    events::{EventRecord, SimulationEvent},
    game_loop::orders::Refusal,
    map::Map,
    order::Order,
    simulation_id::SimulationId,
    spawn::{self, Boarding, SetDown},
    statistics::Statistics,
    visibility::{self, Senses, Sighting},
};

//
// ─── Boarding ───────────────────────────────────────────────────────────────
//

#[test]
fn unit_boards_transport_and_leaves_map_and_selection() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);

    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 40);

    assert_eq!(utils::passengers_of(app.world(), wagon), [rifleman_id]);
    assert!(entity_def::hidden(app.world(), rifleman));
    assert_eq!(
        app.world()
            .get::<BoardedComponent>(rifleman)
            .map(|boarded| boarded.holder),
        Some(wagon_id)
    );
    assert!(
        !utils::selection(&app).contains(&rifleman_id),
        "a unit off the map leaves the selection"
    );
}

#[test]
fn boarded_passenger_holds_no_location_and_marker_keeps_where_it_went_in() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    assert!(
        app.world().get::<LocationComponent>(rifleman).is_none(),
        "an entity off the map stands nowhere"
    );
    // Already in reach at (11, 10), it boarded without a step.
    let went_in = &app
        .world()
        .get::<HiddenComponent>(rifleman)
        .expect("a passenger is hidden")
        .went_in;
    assert_eq!(went_in.position, utils::pos(11, 10));
    let went_in_facing = went_in.facing;

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(!entity_def::hidden(app.world(), rifleman));
    // Beside the wagon, on the ring's first free cell: a ride's end leans
    // nowhere in particular.
    assert_eq!(
        utils::cell_of(app.world(), rifleman),
        CellPos::new(10, 9),
        "set back down beside the holder"
    );
    assert_eq!(
        utils::facing_of(app.world(), rifleman),
        went_in_facing,
        "comes back facing the way it went in"
    );
}

#[test]
fn order_queued_behind_boarding_is_dropped_once_aboard() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(15, 10),
            flush: false,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    utils::run_ticks(&mut app, 3);

    // An entity aboard runs nothing of its own: the walk queued behind the
    // boarding is dropped rather than started from no cell.
    assert!(entity_def::boarded(app.world(), rifleman));
    assert!(utils::order_queue_is_empty(app.world_mut(), rifleman));
}

#[test]
fn riding_trainer_keeps_paid_production_across_ride() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (drillmaster, drillmaster_id) = utils::create_owned(&mut app, "drillmaster", 11, 10, 0);
    utils::grant_gold(&mut app, 10);

    // Boards first, with a rifleman queued behind the boarding and paid for.
    utils::send_to(&mut app, drillmaster_id, wagon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: drillmaster_id,
            type_name: "rifleman".into(),
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    assert!(entity_def::hidden(app.world(), drillmaster));
    assert_eq!(utils::gold(app.world()), 0);

    // Aboard, the production holds: nothing is trained and nothing refunded.
    utils::run_ticks(&mut app, 20);
    let orders = entity_def::orders(app.world(), drillmaster);
    assert!(matches!(orders.as_slice(), [Order::Train]));
    assert_eq!(utils::count_of_type(app.world_mut(), "rifleman"), 0);

    // Set down, the trainer finishes what it was paid for.
    utils::push_command(
        &mut app,
        PlayerCommand::Unload {
            holder: wagon_id,
            at: None,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);
    assert!(!entity_def::hidden(app.world(), drillmaster));
    assert!(entity_def::orders(app.world(), drillmaster).is_empty());
    assert_eq!(utils::count_of_type(app.world_mut(), "rifleman"), 1);
}

#[test]
fn loaded_trainer_keeps_paid_production_across_ride() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (drillmaster, drillmaster_id) = utils::create_owned(&mut app, "drillmaster", 11, 10, 0);
    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: drillmaster_id,
            type_name: "rifleman".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The wagon takes the trainer in mid-production.
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: drillmaster_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    assert_eq!(utils::gold(app.world()), 0);
    let orders = entity_def::orders(app.world(), drillmaster);
    assert!(matches!(orders.as_slice(), [Order::Train]));
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "rifleman"), 0);

    // Set down, the trainer finishes what it was paid for.
    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, 20);
    assert!(entity_def::orders(app.world(), drillmaster).is_empty());
    assert_eq!(utils::count_of_type(app.world_mut(), "rifleman"), 1);
}

#[test]
fn passenger_production_can_be_canceled_and_refunded_mid_ride() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (drillmaster, drillmaster_id) = utils::create_owned(&mut app, "drillmaster", 11, 10, 0);
    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: drillmaster_id,
            type_name: "rifleman".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: drillmaster_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    assert_eq!(utils::gold(app.world()), 0);

    // Production needs no position to be called off: the owner cancels it
    // aboard and is paid back the rifleman's 10.
    utils::push_command(
        &mut app,
        PlayerCommand::CancelTrain {
            trainer: drillmaster_id,
            slot: 0,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), drillmaster));
    assert_eq!(utils::gold(app.world()), 10);

    // Set down, it trains nothing.
    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, 20);
    assert_eq!(utils::count_of_type(app.world_mut(), "rifleman"), 0);
}

#[test]
fn passenger_research_can_be_canceled_and_refunded_mid_ride() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (drillmaster, drillmaster_id) = utils::create_owned(&mut app, "drillmaster", 11, 10, 0);
    let drill = utils::research_id(&app, "drill");
    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::StartResearch {
            researcher: drillmaster_id,
            research: drill,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: drillmaster_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    // 10 granted − 10 for the drill.
    assert_eq!(utils::gold(app.world()), 0);

    // The topic is called off aboard and its 10 paid back.
    utils::push_command(
        &mut app,
        PlayerCommand::CancelResearch {
            researcher: drillmaster_id,
            research: drill,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), drillmaster));
    assert_eq!(utils::gold(app.world()), 10);
    assert!(entity_def::orders(app.world(), drillmaster).is_empty());
}

#[test]
fn unit_loaded_mid_walk_stands_once_set_down() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (grunt, grunt_id) = utils::create_owned(&mut app, "grunt", 11, 10, 0);
    utils::select(&mut app, grunt_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 20),
            flush: true,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: grunt_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 20);

    // The walk ends with the boarding; nothing of it waits out the ride.
    assert!(entity_def::orders(app.world(), grunt).is_empty());

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let set_down = utils::cell_of(app.world(), grunt);
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::cell_of(app.world(), grunt), set_down);
    assert!(entity_def::orders(app.world(), grunt).is_empty());
}

#[test]
fn worker_loaded_off_job_leaves_job_before_boarding() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (orderly, orderly_id) = utils::create_owned(&mut app, "orderly", 11, 10, 0);
    utils::wound(&mut app, wagon, "20");
    utils::select(&mut app, orderly_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: wagon_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // 40 + 2 × 5: two ticks of mending from the open.
    assert_eq!(utils::health_as_u32(&app, wagon), 50);

    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: orderly_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    let mended_to = utils::health_as_u32(&app, wagon);
    utils::run_ticks(&mut app, 2);

    // The job ends before the boarding, so no cancel sets a boarded body back
    // on the grid, and the mending stops with it.
    assert!(app.world().get::<LocationComponent>(orderly).is_none());
    assert!(entity_def::boarded(app.world(), orderly));
    assert!(utils::order_queue_is_empty(app.world_mut(), orderly));
    assert_eq!(utils::health_as_u32(&app, wagon), mended_to);
}

#[test]
fn patient_that_boards_mid_repair_ends_mender_order() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (orderly, orderly_id) = utils::create_owned(&mut app, "orderly", 12, 10, 0);
    utils::wound(&mut app, rifleman, "20");
    utils::select(&mut app, orderly_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: rifleman_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    // 10 + 2 × 5: two ticks of mending.
    assert_eq!(utils::health_as_u32(&app, rifleman), 20);

    // The patient climbs into the wagon beside it: gone from the map, gone
    // from the order, with the mender standing where it worked.
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    let boarded_at = utils::health_as_u32(&app, rifleman);
    utils::run_ticks(&mut app, 1);

    assert!(utils::order_queue_is_empty(app.world_mut(), orderly));
    assert_eq!(utils::cell_of(app.world(), orderly), CellPos::new(12, 10));
    // Mended no further once aboard.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::health_as_u32(&app, rifleman), boarded_at);
}

#[test]
fn load_refuses_unit_changing_form() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (pupa, pupa_id) = utils::create_owned(&mut app, "pupa", 11, 10, 0);
    utils::select(&mut app, pupa_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Morph {
            type_name: "moth".into(),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::changing(app.world(), pupa));

    // A body changing form boards nothing: the hold turns it away and the
    // change goes on where it stands.
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: pupa_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 4);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(pupa_id)
            .is_some()
    );
    assert!(!entity_def::hidden(app.world(), pupa));
    assert_eq!(utils::passengers_of(app.world(), wagon), []);
}

#[test]
fn passenger_aboard_is_seen_by_nobody() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);
    // The rival stands beside the wagon, so its cell is in the rival's sight.
    utils::create_owned(&mut app, "rifleman", 11, 11, 2);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 40);
    utils::run_ticks(&mut app, 1);

    // The wagon is in plain sight; what rides inside it is off the map, and
    // sight reaches nothing there — not the rival's, not the owner's, not an
    // ally's. The owner knows its passenger from the wagon's cargo list.
    let world = app.world();
    assert_eq!(
        visibility::sighting(world, 2, wagon, Senses::SeatDeclared),
        Sighting::Seen
    );
    for player in [2, 0, 1] {
        assert_eq!(
            visibility::sighting(world, player, rifleman, Senses::SeatDeclared),
            Sighting::Unseen,
            "player {player}"
        );
    }
}

#[test]
fn boarder_turns_away_from_holder_switched_off_on_way() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 16, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);

    // A daze stops the wagon working while the rifleman is on the way: it
    // turns away on arrival, and nobody boards a holder that is switched off.
    let dazed = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            "dazed",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::For(30),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
    utils::apply_buff(app.world_mut(), wagon, dazed);
    utils::run_ticks(&mut app, 20);

    assert!(!entity_def::hidden(app.world(), rifleman));
    assert_eq!(utils::passengers_of(app.world(), wagon), []);
    assert!(utils::order_queue_is_empty(app.world_mut(), rifleman));
}

#[test]
#[should_panic(expected = "a passenger boards a holder alive, on the map and operating")]
fn board_panics_for_dying_holder() {
    let mut app = utils::transport_app();
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, _) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    spawn::destroy_entity(app.world_mut(), wagon);
    spawn::board(app.world_mut(), rifleman, wagon);
}

#[test]
fn loader_turns_away_from_target_switched_off_on_way() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (grunt, grunt_id) = utils::create_owned(&mut app, "grunt", 16, 10, 0);
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: grunt_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);

    // A daze stops the grunt working while the wagon is on its way: the
    // wagon turns away, and nobody is loaded who is switched off.
    let dazed = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            "dazed",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::For(30),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
    utils::apply_buff(app.world_mut(), grunt, dazed);
    utils::run_ticks(&mut app, 20);

    assert!(!entity_def::hidden(app.world(), grunt));
    assert_eq!(utils::passengers_of(app.world(), wagon), []);
    assert!(utils::order_queue_is_empty(app.world_mut(), wagon));
}

#[test]
fn untransportable_unit_does_not_board() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (civilian, civilian_id) = utils::create_owned(&mut app, "civilian", 13, 10, 0);

    utils::send_to(&mut app, civilian_id, wagon_id);
    utils::run_ticks(&mut app, 30);

    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(
        !entity_def::hidden(app.world(), civilian),
        "a unit without cargo_size stays in the open"
    );
}

#[test]
fn board_refuses_unit_that_cannot_ride_and_leaves_it_standing() {
    let mut app = utils::transport_app();
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (civilian, _) = utils::create_owned(&mut app, "civilian", 11, 10, 0);
    let (bunker, _) = utils::create_owned(&mut app, "bunker", 14, 10, 0);
    let (grunt, _) = utils::create_owned(&mut app, "grunt", 13, 12, 0);

    // A civilian declares no cargo_size; a bunker carries no grunt.
    assert_eq!(
        spawn::board(app.world_mut(), civilian, wagon),
        Boarding::Refused(Refusal::Incapable)
    );
    assert_eq!(
        spawn::board(app.world_mut(), grunt, bunker),
        Boarding::Refused(Refusal::TargetUnfit)
    );

    // A refused passenger is untouched.
    assert_eq!(utils::cell_of(app.world(), civilian), CellPos::new(11, 10));
    assert_eq!(utils::cell_of(app.world(), grunt), CellPos::new(13, 12));
    assert!(!entity_def::hidden(app.world(), civilian));
    assert!(!entity_def::hidden(app.world(), grunt));
    assert_eq!(utils::passengers_of(app.world(), wagon), []);
    assert_eq!(utils::passengers_of(app.world(), bunker), []);
}

#[test]
fn admission_list_matches_type_name() {
    let mut app = utils::transport_app();
    // The bunker carries "rifleman" by type name; a grunt's "infantry" tag is
    // not on its list.
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 13, 10, 0);
    let (grunt, grunt_id) = utils::create_owned(&mut app, "grunt", 13, 12, 0);

    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::send_to(&mut app, grunt_id, bunker_id);
    utils::run_ticks(&mut app, 30);

    assert_eq!(utils::passengers_of(app.world(), bunker), [rifleman_id]);
    assert!(!entity_def::hidden(app.world(), grunt));
}

#[test]
fn debuff_sealing_hold_turns_boarder_away() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);

    // Capacity has no fold floor, so a debuff can seal the hold entirely.
    let sealed = utils::register_entity_buff(
        &mut app,
        "sealed_hold",
        EntityStatId::CARGO_CAPACITY,
        ModifierOp::FlatAdd,
        "-10",
        None,
    );
    utils::apply_buff(app.world_mut(), wagon, sealed);

    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_ticks(&mut app, 20);

    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(
        !entity_def::hidden(app.world(), rifleman),
        "a sealed hold admits nobody"
    );
}

#[test]
fn full_transport_turns_boarder_away() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    // Two grunts at cargo size 2 each fill the four slots.
    let (_, first_id) = utils::create_owned(&mut app, "grunt", 12, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "grunt", 12, 11, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 12, 0);

    utils::send_to(&mut app, first_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 40);
    utils::send_to(&mut app, second_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 2, 40);

    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_ticks(&mut app, 30);

    assert_eq!(utils::passengers_of(app.world(), wagon).len(), 2);
    assert!(
        !entity_def::hidden(app.world(), rifleman),
        "no slot left for even a one-slot passenger"
    );
}

#[test]
fn own_boarding_policy_turns_away_allied_unit() {
    let mut app = utils::transport_app();
    // Player 1 is allied with the local player 0; the wagon admits own only.
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 1);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 13, 10, 0);

    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_ticks(&mut app, 30);

    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(!entity_def::hidden(app.world(), rifleman));
}

#[test]
fn allies_boarding_policy_admits_allied_unit() {
    let mut app = utils::transport_app();
    let (ferry, ferry_id) = utils::create_owned(&mut app, "ferry", 10, 10, 1);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 13, 10, 0);

    utils::send_to(&mut app, rifleman_id, ferry_id);
    utils::run_until_aboard(&mut app, ferry, 1, 40);

    assert_eq!(utils::passengers_of(app.world(), ferry), [rifleman_id]);
}

#[test]
fn allied_boarding_ferry_loads_nobody_of_its_ally() {
    let mut app = utils::transport_app();
    let (ferry, ferry_id) = utils::create_owned(&mut app, "ferry", 10, 10, 0);
    // Player 1 is player 0's ally; it may board the ferry itself.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 1);

    utils::load(&mut app, ferry_id, rifleman_id);
    utils::run_ticks(&mut app, utils::APPLY + 5);

    assert!(utils::passengers_of(app.world(), ferry).is_empty());
    assert!(!entity_def::hidden(app.world(), rifleman));
}

#[test]
fn load_period_meters_boardings() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    // Both stand inside the load range already, so they arrive together.
    let (_, first_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "rifleman", 11, 11, 0);

    utils::select(&mut app, first_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: second_id,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: wagon_id,
            flush: true,
        },
    );

    utils::run_until_aboard(&mut app, wagon, 1, 10);
    assert_eq!(
        utils::passengers_of(app.world(), wagon).len(),
        1,
        "the boarding cooldown holds the second arrival outside"
    );
    // The second boards once the three-tick loading period has passed.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::passengers_of(app.world(), wagon).len(), 2);
}

#[test]
fn explicit_board_command_boards_mixed_selection() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    let (civilian, civilian_id) = utils::create_owned(&mut app, "civilian", 12, 11, 0);

    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: civilian_id,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: wagon_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 30);

    assert_eq!(
        utils::passengers_of(app.world(), wagon),
        [rifleman_id],
        "the eligible unit boards"
    );
    assert!(entity_def::hidden(app.world(), rifleman));
    assert!(
        !entity_def::hidden(app.world(), civilian),
        "the ineligible unit drops the order and stays out"
    );
}

#[test]
fn explicit_follow_command_tails_own_transporter() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);

    // The smart click would read this pairing as boarding; the explicit
    // command keeps the rifleman outside, tailing the wagon.
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Follow {
            target: wagon_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(!entity_def::hidden(app.world(), rifleman));
    assert!(
        utils::within(app.world_mut(), rifleman, wagon, 2),
        "the follower stays close without climbing in"
    );
}

#[test]
fn transport_fetches_targeted_unit_aboard() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 5, 5, 0);
    let (grunt, grunt_id) = utils::create_owned(&mut app, "grunt", 14, 5, 0);

    // The grunt is walking its own way; the wagon is sent to fetch it.
    utils::select(&mut app, grunt_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(14, 12),
            flush: true,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: grunt_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, wagon, 1, 80);

    assert_eq!(utils::passengers_of(app.world(), wagon), [grunt_id]);
    assert!(entity_def::hidden(app.world(), grunt));
    assert!(
        app.world()
            .get::<OrderQueueComponent>(grunt)
            .is_some_and(|queue| queue.front().is_none()),
        "the fetched unit's own orders are canceled"
    );
}

#[test]
fn bunker_calls_rifleman_beyond_its_load_range_over_to_board_it() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    // Four cells off, beyond the one-cell load range of a bunker that cannot
    // walk to it.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);

    utils::load(&mut app, bunker_id, rifleman_id);
    utils::run_until_aboard(&mut app, bunker, 1, 40);

    assert!(entity_def::hidden(app.world(), rifleman));
}

#[test]
fn rifleman_called_over_mid_chase_drops_its_chase_walk() {
    let mut app = utils::transport_app();
    let (_, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    // Created after the bunker, so processed after it in the tick of the call.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);
    let (_, leader_id) = utils::create_owned(&mut app, "rifleman", 14, 25, 0);
    // Told to follow and called over in the same tick: the follow's first
    // step pushes a walk toward the leader after the call marked the queue.
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Follow {
            target: leader_id,
            flush: true,
        },
    );
    utils::load(&mut app, bunker_id, rifleman_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Walking to the bunker at (10, 10) to board it, not toward the leader.
    let orders = entity_def::orders(app.world(), rifleman);
    let Order::Move { target, .. } = orders[0] else {
        panic!("the boarding walks first, got {orders:?}");
    };
    assert_eq!(target, utils::pos(10, 10));
    assert_eq!(orders[1].board_target(), Some(bunker_id));
}

#[test]
fn rifleman_called_over_by_two_bunkers_boards_first_caller() {
    let mut app = utils::transport_app();
    let (first, first_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "bunker", 18, 10, 0);
    // Four cells from each, beyond either's one-cell load range.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);

    utils::load(&mut app, first_id, rifleman_id);
    utils::load(&mut app, second_id, rifleman_id);
    utils::run_until_aboard(&mut app, first, 1, 40);

    assert!(entity_def::hidden(app.world(), rifleman));
    assert!(utils::passengers_of(app.world(), second).is_empty());
}

#[test]
fn load_refuses_ineligible_unit() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 5, 5, 0);
    let (civilian, civilian_id) = utils::create_owned(&mut app, "civilian", 8, 5, 0);

    utils::push_command(
        &mut app,
        PlayerCommand::Load {
            holder: wagon_id,
            target: civilian_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(
        !entity_def::hidden(app.world(), civilian),
        "a unit without cargo_size is not fetched"
    );
}

//
// ─── Shots in flight ────────────────────────────────────────────────────────
//

#[test]
fn shot_following_target_that_boards_hits_nobody() {
    let mut app = utils::transport_app();
    let (_, archer_id) = utils::create_owned(&mut app, "archer", 19, 10, 0);
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 11, 20, 0);
    let rifleman = board_walker_under_fire(&mut app, archer_id, wagon);
    let (went_in_grunt, _) = utils::create_owned(&mut app, "grunt", 11, 12, 0);
    let (fired_at_grunt, _) = utils::create_owned(&mut app, "grunt", 11, 10, 0);
    utils::run_ticks(&mut app, 10);

    // A shot that follows its target hits that target or nobody: the arrow
    // lands with no one to strike, on neither cell's occupant.
    assert_eq!(utils::health_as_u32(&app, went_in_grunt), 20);
    assert_eq!(utils::health_as_u32(&app, fired_at_grunt), 20);
    // 30, the rifleman's full health: aboard, nothing reached it.
    assert_eq!(utils::health_as_u32(&app, rifleman), 30);
}

#[test]
fn burst_of_shot_following_target_that_boards_lands_where_target_went_in() {
    let mut app = utils::transport_app();
    let (_, slinger_id) = utils::create_owned(&mut app, "slinger", 19, 10, 0);
    let (wagon, _) = utils::create_owned(&mut app, "wagon", 11, 20, 0);
    let rifleman = board_walker_under_fire(&mut app, slinger_id, wagon);
    let (went_in_grunt, _) = utils::create_owned(&mut app, "grunt", 11, 12, 0);
    let (fired_at_grunt, _) = utils::create_owned(&mut app, "grunt", 11, 10, 0);
    utils::run_ticks(&mut app, 10);

    // No direct victim, but the stone bursts where its target went in:
    // 20 − 10 × 0.5 = 15 on that cell, nothing two cells off.
    assert_eq!(utils::health_as_u32(&app, went_in_grunt), 15);
    assert_eq!(utils::health_as_u32(&app, fired_at_grunt), 20);
    // 30, the rifleman's full health: aboard, nothing reached it.
    assert_eq!(utils::health_as_u32(&app, rifleman), 30);
}

#[test]
fn shot_following_target_that_left_map_hits_it_no_more() {
    let mut app = utils::transport_app();
    let (_, archer_id) = utils::create_owned(&mut app, "archer", 19, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::attack(&mut app, archer_id, rifleman_id);
    // The arrow leaves on tick 3 and lands on tick 11.
    utils::run_ticks(&mut app, 6);

    // The target goes off the map and straight back while the arrow flies:
    // the arrow lost it when it went, and lands on nobody.
    spawn::hide_entity(app.world_mut(), rifleman, Inside::Nothing);
    assert_eq!(
        spawn::place_back_beside(
            app.world_mut(),
            rifleman,
            Beside::Footprint(CellRect::new(CellPos::new(11, 10), CellSize::ONE)),
            Lean::WentIn
        ),
        SetDown::Placed
    );
    utils::select(&mut app, archer_id);
    utils::push_command(&mut app, PlayerCommand::Stop);
    utils::run_ticks(&mut app, 10);
    // 30, the rifleman's full health: the arrow hit nobody.
    assert_eq!(utils::health_as_u32(&app, rifleman), 30);
}

#[test]
fn burst_of_shot_at_dying_target_lands_where_it_lies() {
    let mut app = utils::transport_app();
    let (_, slinger_id) = utils::create_owned(&mut app, "slinger", 19, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, killer_id) = utils::create_owned(&mut app, "grunt", 30, 30, 2);
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 19),
            flush: true,
        },
    );
    utils::attack(&mut app, slinger_id, rifleman_id);
    // The stone leaves sent to (11, 10); the walker, ten ticks on, stands on
    // (11, 14) and is killed there, two ticks before the stone lands.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(11, 14));
    spawn::despawn_killed(
        app.world_mut(),
        rifleman,
        killer_id,
        Some(2),
        Slain::Remains,
    );
    let (beside, _) = utils::create_owned(&mut app, "grunt", 12, 14, 0);
    let (fired_at, _) = utils::create_owned(&mut app, "grunt", 11, 10, 0);
    utils::run_ticks(&mut app, 2);

    // The stone bursts where its dying target lies: 20 − 10 × 0.5 = 15 a cell
    // from it, nothing at the cell it was sent to.
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .any(rifleman_id)
            .is_some(),
        "still lying there, dying"
    );
    assert_eq!(utils::health_as_u32(&app, beside), 15);
    assert_eq!(utils::health_as_u32(&app, fired_at), 20);
}

//
// ─── Unloading ──────────────────────────────────────────────────────────────
//

#[test]
fn unload_period_meters_exits_in_id_order() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (first, first_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "rifleman", 11, 11, 0);
    utils::send_to(&mut app, first_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    utils::send_to(&mut app, second_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 2, 10);

    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(
        !entity_def::hidden(app.world(), first),
        "the lower id steps out first"
    );
    assert!(entity_def::hidden(app.world(), second));

    // The second follows after the two-tick unloading period.
    utils::run_ticks(&mut app, 2);
    assert!(!entity_def::hidden(app.world(), second));
    assert!(utils::passengers_of(app.world(), wagon).is_empty());
    assert!(!entity_def::boarded(app.world(), first));
}

#[test]
fn fractional_unload_period_rounds_up() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (first, first_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "rifleman", 11, 11, 0);
    utils::send_to(&mut app, first_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    utils::send_to(&mut app, second_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 2, 10);
    // 2 − 0.75 = 1.25 ticks between exits, rounded up to 2.
    let hastened = utils::register_entity_buff(
        &mut app,
        "hastened_exit",
        EntityStatId::UNLOAD_PERIOD,
        ModifierOp::FlatAdd,
        "-0.75",
        None,
    );
    utils::apply_buff(app.world_mut(), wagon, hastened);

    // The first steps out the tick the unload applies; the second, two
    // ticks later.
    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(!entity_def::hidden(app.world(), first));
    assert!(entity_def::hidden(app.world(), second));

    utils::run_ticks(&mut app, 1);
    assert!(entity_def::hidden(app.world(), second));
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::hidden(app.world(), second));
}

#[test]
fn fractional_load_period_rounds_up() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, first_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "rifleman", 11, 11, 0);
    // 3 − 1.75 = 1.25 ticks between boardings, rounded up to 2.
    let hastened = utils::register_entity_buff(
        &mut app,
        "hastened_boarding",
        EntityStatId::LOAD_PERIOD,
        ModifierOp::FlatAdd,
        "-1.75",
        None,
    );
    utils::apply_buff(app.world_mut(), wagon, hastened);
    utils::select(&mut app, first_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SelectById {
            id: second_id,
            mode: SelectMode::Add,
        },
    );
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: wagon_id,
            flush: true,
        },
    );

    utils::run_until_aboard(&mut app, wagon, 1, 10);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::passengers_of(app.world(), wagon).len(), 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::passengers_of(app.world(), wagon).len(), 2);
}

#[test]
fn unmetered_holder_empties_in_one_tick() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (first, first_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "rifleman", 12, 11, 0);
    utils::send_to(&mut app, first_id, bunker_id);
    utils::send_to(&mut app, second_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 2, 40);

    utils::unload(&mut app, bunker_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(!entity_def::hidden(app.world(), first));
    assert!(!entity_def::hidden(app.world(), second));
    assert!(utils::passengers_of(app.world(), bunker).is_empty());
}

#[test]
fn unload_at_point_walks_there_and_sends_passengers_on() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 5, 5, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 6, 5, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    utils::unload(&mut app, wagon_id, Some(utils::pos(20, 5)));
    utils::run_ticks(&mut app, 80);

    assert!(!entity_def::hidden(app.world(), rifleman));
    let wagon_cell = utils::cell_of(app.world_mut(), wagon);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(wagon_cell, CellPos::new(20, 5), 1),
        "the wagon walked into unload range of the point, stands at {wagon_cell:?}"
    );
    // The freed passenger marches on toward the point itself.
    let rifleman_cell = utils::cell_of(app.world_mut(), rifleman);
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
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    utils::unload(&mut app, bunker_id, Some(utils::pos(20, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(
        !entity_def::hidden(app.world(), rifleman),
        "a holder that cannot walk lets everyone out where it stands"
    );
    utils::run_ticks(&mut app, 60);
    let rifleman_cell = utils::cell_of(app.world_mut(), rifleman);
    assert!(
        app.world()
            .resource::<Map>()
            .projection()
            .in_range(rifleman_cell, CellPos::new(20, 10), 1),
        "the passenger still walks to the point, stands at {rifleman_cell:?}"
    );
}

#[test]
fn passenger_unloaded_toward_point_steps_out_on_its_side() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    // It boards from the bunker's west side.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 8, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    // Unloaded toward a point east: of the ring around the bunker's (10, 10)
    // 2×2, the free cell nearest (20, 10) is (12, 10).
    utils::unload(&mut app, bunker_id, Some(utils::pos(20, 10)));
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(12, 10));
}

#[test]
fn passenger_unloaded_without_point_steps_out_toward_rally_point() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    // It boards from the bunker's west side.
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 8, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);
    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: bunker_id,
            target: Some(RallyTarget::Position(utils::pos(20, 10))),
        },
    );

    // Unloaded with no point of its own: it steps out on the side of the
    // rally point east, (12, 10), the ring cell nearest (20, 10).
    utils::unload(&mut app, bunker_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(12, 10));
}

#[test]
fn rally_point_sends_unloaded_passenger_on() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    utils::push_command(
        &mut app,
        PlayerCommand::SetRallyPoint {
            entity: wagon_id,
            target: Some(RallyTarget::Position(utils::pos(18, 10))),
        },
    );
    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, 60);

    let rifleman_cell = utils::cell_of(app.world_mut(), rifleman);
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
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::unload(&mut app, wagon_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 4);

    assert!(
        entity_def::hidden(app.world(), rifleman),
        "a boxed-in exit keeps the passenger aboard"
    );
    assert_eq!(utils::passengers_of(app.world(), wagon), [rifleman_id]);

    // Free one cell beside the wagon; the held exit lands on it.
    utils::free_cell(&mut app, CellPos::new(11, 10));
    utils::run_ticks(&mut app, 2);
    assert!(!entity_def::hidden(app.world(), rifleman));
    assert!(utils::passengers_of(app.world(), wagon).is_empty());
}

//
// ─── Holder death ───────────────────────────────────────────────────────────
//

#[test]
fn destroyed_wagon_takes_passengers_with_it() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    spawn::destroy_entity(app.world_mut(), wagon);
    utils::run_ticks(&mut app, 5);

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
fn passenger_destroyed_with_holder_dies_where_holder_stands() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);
    app.world_mut().resource_mut::<EventRecord>().clear();

    spawn::destroy_entity(app.world_mut(), wagon);

    // The wagon's own cell, not the one the rifleman boarded from.
    let died_at: Vec<FixedUVec2> = app
        .world()
        .resource::<EventRecord>()
        .events()
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityDied {
                entity, position, ..
            } if *entity == rifleman_id => Some(*position),
            _ => None,
        })
        .collect();
    assert_eq!(died_at, [utils::pos(10, 10)]);
}

#[test]
fn destroyed_bunker_ejects_passengers() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    spawn::destroy_entity(app.world_mut(), bunker);
    utils::run_ticks(&mut app, 1);

    assert!(
        !entity_def::hidden(app.world(), rifleman),
        "an eject-fated holder puts its passengers back on the map"
    );
    assert!(!entity_def::boarded(app.world(), rifleman));
    assert!(
        utils::within(app.world_mut(), rifleman, bunker, 2),
        "ejected beside the holder's footprint"
    );
}

#[test]
fn ejected_passenger_without_room_dies() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    spawn::destroy_entity(app.world_mut(), bunker);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(rifleman_id)
            .is_none(),
        "a passenger the ring scan cannot place dies with its holder"
    );
    assert!(
        !entity_def::pending_reveal(app.world(), rifleman),
        "nothing lingers hidden waiting for a holder that is gone"
    );
}

#[test]
fn holder_destroyed_inside_job_sets_passengers_down_beside_job() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (ambulance, ambulance_id) = utils::create_owned(&mut app, "ambulance", 13, 10, 0);
    let (rifleman, _) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, ambulance),
        Boarding::Boarded
    );
    utils::wound(&mut app, bunker, "50");
    utils::select(&mut app, ambulance_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: bunker_id,
            flush: true,
        },
    );
    run_until_hidden(&mut app, ambulance, 10);

    spawn::destroy_entity(app.world_mut(), ambulance);

    // The holder is inside the bunker, so the rider comes out beside the
    // bunker, on its ring's first free cell, (10, 9).
    assert!(!entity_def::boarded(app.world(), rifleman));
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(10, 9));
}

//
// ─── Capture ────────────────────────────────────────────────────────────────
//

#[test]
fn captured_holder_with_destroy_fate_loses_passengers_its_new_owner_bars() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "rifleman", 20, 20, 2);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    // The wagon boards its owner's side only; handed to player 2, it no
    // longer admits player 0's rifleman, and its fate for the lost is death.
    spawn::change_owner(app.world_mut(), wagon, 2, taker_id, Lean::Nothing);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(rifleman_id)
            .is_none()
    );
    assert_eq!(utils::passengers_of(app.world(), wagon), []);

    // Lost to its owner, a kill for nobody: the taker fired no shot.
    utils::run_ticks(&mut app, 1);
    let rifleman_type = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("rifleman")
        .expect("test content registers the type");
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(rifleman_type), 1);
    assert_eq!(statistics.player(2).killed(rifleman_type), 0);
}

#[test]
fn captured_holder_with_eject_fate_sets_barred_passengers_down() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "rifleman", 20, 20, 2);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    spawn::change_owner(app.world_mut(), bunker, 2, taker_id, Lean::Nothing);

    assert!(!entity_def::boarded(app.world(), rifleman));
    assert!(!entity_def::hidden(app.world(), rifleman));
    // Beside the bunker, on its ring's first free cell, (10, 9): a ride's end
    // leans nowhere in particular.
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(10, 9));
    assert_eq!(utils::passengers_of(app.world(), bunker), []);
}

#[test]
fn captured_holder_keeps_passengers_its_new_owner_admits() {
    let mut app = utils::transport_app();
    let (raft, raft_id) = utils::create_owned(&mut app, "raft", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (_, taker_id) = utils::create_owned(&mut app, "rifleman", 20, 20, 2);
    utils::send_to(&mut app, rifleman_id, raft_id);
    utils::run_until_aboard(&mut app, raft, 1, 10);

    // A raft boards anyone, so the new owner admits what is already aboard.
    spawn::change_owner(app.world_mut(), raft, 2, taker_id, Lean::Nothing);

    assert_eq!(
        app.world()
            .get::<BoardedComponent>(rifleman)
            .map(|boarded| boarded.holder),
        Some(raft_id)
    );
    assert_eq!(utils::passengers_of(app.world(), raft), [rifleman_id]);
}

//
// ─── Garrison attack ────────────────────────────────────────────────────────
//

#[test]
fn garrisoned_rifleman_fires_from_bunker() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    // A hostile in weapon range of the bunker, and one beyond it.
    let (near, _) = utils::create_owned(&mut app, "grunt", 13, 10, 2);
    let (far, _) = utils::create_owned(&mut app, "grunt", 20, 10, 2);
    utils::run_ticks(&mut app, 40);

    // 20 − 10: one shot lands, then the unarmed grunt flees past the
    // rifleman's range of 3.
    assert_eq!(
        utils::health_as_u32(&app, near),
        10,
        "the passenger's own weapon works the target from the holder"
    );
    assert_eq!(
        utils::health_as_u32(&app, far),
        20,
        "a target beyond the passenger's range is untouched"
    );
}

#[test]
fn garrison_fire_stops_once_unloaded() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);

    utils::unload(&mut app, bunker_id, None);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(!entity_def::hidden(app.world(), rifleman));
    assert!(
        app.world().get::<GarrisonFireComponent>(rifleman).is_none(),
        "the firing state leaves with the passenger"
    );
}

#[test]
fn wagon_passengers_do_not_fire() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 10, 10, 0);
    let (_, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 10);

    let (near, _) = utils::create_owned(&mut app, "grunt", 13, 10, 2);
    utils::run_ticks(&mut app, 40);

    assert_eq!(
        utils::health_as_u32(&app, near),
        20,
        "a sheltering holder keeps its cargo quiet"
    );
}

#[test]
fn garrison_of_holder_inside_job_holds_its_fire() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (ambulance, ambulance_id) = utils::create_owned(&mut app, "ambulance", 13, 10, 0);
    let (rifleman, _) = utils::create_owned(&mut app, "rifleman", 14, 10, 0);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, ambulance),
        Boarding::Boarded
    );
    utils::wound(&mut app, bunker, "50");
    // In the rifleman's reach of the ambulance, out of the bunker's.
    let (grunt, _) = utils::create_owned(&mut app, "grunt", 15, 12, 2);
    utils::select(&mut app, ambulance_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: bunker_id,
            flush: true,
        },
    );
    run_until_hidden(&mut app, ambulance, 10);

    // Off the map there is nothing to fire from: the grunt is untouched for
    // as long as the mending lasts.
    utils::run_ticks(&mut app, 5);
    assert!(entity_def::hidden(app.world(), ambulance));
    assert_eq!(utils::health_as_u32(&app, grunt), 20);

    // Once the bunker is whole the ambulance stands again beside it, on the
    // side it went in from, (12, 10) — three cells from the grunt — and its
    // rifleman fires from it.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::cell_of(app.world(), ambulance), CellPos::new(12, 10));
    utils::run_ticks(&mut app, 40);
    // 20 − 10: one shot lands, then the unarmed grunt flees past the
    // rifleman's range of 3.
    assert_eq!(utils::health_as_u32(&app, grunt), 10);
}

//
// ─── Workers inside a job ───────────────────────────────────────────────────
//

#[test]
fn worker_inside_walking_patient_is_with_patient_and_comes_out_beside_it() {
    let mut app = utils::transport_app();
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (surgeon, surgeon_id) = utils::create_owned(&mut app, "surgeon", 12, 10, 0);
    utils::wound(&mut app, rifleman, "20");
    utils::select(&mut app, surgeon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: rifleman_id,
            flush: true,
        },
    );
    run_until_hidden(&mut app, surgeon, 10);
    let went_in = app
        .world()
        .get::<HiddenComponent>(surgeon)
        .expect("the surgeon works hidden")
        .went_in
        .position;

    // The patient walks on with the surgeon inside it: the surgeon is
    // wherever its patient is, and comes out beside it once it is whole.
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 16),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 8);
    let patient_at = entity_def::position(app.world(), rifleman);
    assert_ne!(patient_at, went_in);
    assert_eq!(entity_def::whereabouts(app.world(), surgeon), patient_at);

    // 10 + 1 a tick: whole within the twenty ticks.
    utils::run_ticks(&mut app, 20);
    assert!(!entity_def::hidden(app.world(), surgeon));
    // The rifleman stands on (11, 16), its walk done; of the cells around it
    // the free one nearest where the surgeon went in, (12, 10), is (12, 15).
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(11, 16));
    assert_eq!(utils::cell_of(app.world(), surgeon), CellPos::new(12, 15));
}

#[test]
fn worker_inside_patient_killed_comes_out_beside_dying_patient() {
    let mut app = utils::transport_app();
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (surgeon, surgeon_id) = utils::create_owned(&mut app, "surgeon", 12, 10, 0);
    let (_, killer_id) = utils::create_owned(&mut app, "grunt", 30, 30, 2);
    utils::wound(&mut app, rifleman, "20");
    utils::select(&mut app, surgeon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: rifleman_id,
            flush: true,
        },
    );
    run_until_hidden(&mut app, surgeon, 10);
    utils::select(&mut app, rifleman_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(11, 16),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 14);
    assert_eq!(utils::cell_of(app.world(), rifleman), CellPos::new(11, 16));
    assert!(entity_def::hidden(app.world(), surgeon));

    // The patient is killed with the surgeon inside: while it lies dying on
    // (11, 16) the surgeon comes out beside it, on the free cell nearest
    // where it went in, (12, 15), not back at (12, 10).
    spawn::despawn_killed(
        app.world_mut(),
        rifleman,
        killer_id,
        Some(2),
        Slain::Remains,
    );
    // The death sets the surgeon down at once, before any tick runs.
    assert!(!entity_def::hidden(app.world(), surgeon));
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::dying(app.world(), rifleman));
    assert!(!entity_def::hidden(app.world(), surgeon));
    assert_eq!(utils::cell_of(app.world(), surgeon), CellPos::new(12, 15));
}

#[test]
fn worker_inside_patient_that_boards_comes_out_beside_holder() {
    let mut app = utils::transport_app();
    let (wagon, wagon_id) = utils::create_owned(&mut app, "wagon", 11, 15, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 11, 10, 0);
    let (surgeon, surgeon_id) = utils::create_owned(&mut app, "surgeon", 12, 10, 0);
    utils::wound(&mut app, rifleman, "20");
    utils::select(&mut app, surgeon_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Repair {
            target: rifleman_id,
            flush: true,
        },
    );
    run_until_hidden(&mut app, surgeon, 10);

    // The patient walks to the wagon and climbs in with the surgeon inside
    // it: the mending ends, and the surgeon comes out beside the wagon.
    utils::send_to(&mut app, rifleman_id, wagon_id);
    utils::run_until_aboard(&mut app, wagon, 1, 30);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::hidden(app.world(), surgeon));
    // Around the wagon on (11, 15), the free cell nearest where the surgeon
    // went in, (12, 10), is (12, 14).
    assert_eq!(utils::cell_of(app.world(), surgeon), CellPos::new(12, 14));
}

//
// ─── Hidden entities and blasts ─────────────────────────────────────────────
//

#[test]
fn blast_spares_hidden_passenger_where_it_went_in() {
    let mut app = utils::transport_app();
    let (bunker, bunker_id) = utils::create_owned(&mut app, "bunker", 10, 10, 0);
    let (rifleman, rifleman_id) = utils::create_owned(&mut app, "rifleman", 12, 10, 0);
    utils::send_to(&mut app, rifleman_id, bunker_id);
    utils::run_until_aboard(&mut app, bunker, 1, 30);
    let full = utils::health(&app, rifleman);

    // A hostile bombard auto-engages the bunker; its blast covers the cell the
    // rifleman stood on when it boarded.
    utils::create_owned(&mut app, "bombard", 16, 10, 2);
    utils::run_ticks(&mut app, 30);

    let bunker = utils::single_owned_of_type(app.world_mut(), "bunker", 0);
    // 200 − 7 × 20: seven 20-point shells in the 30 ticks.
    assert_eq!(
        utils::health_as_u32(&app, bunker),
        60,
        "the shells land on the bunker itself"
    );
    assert_eq!(
        utils::health(&app, rifleman),
        full,
        "a passenger is beyond any blast's reach"
    );
}

#[test]
fn turret_only_passenger_sits_ride_out() {
    // What fires from inside a holder is the weapon a passenger points itself: a
    // turret is mounted on a body standing somewhere, and a passenger stands
    // nowhere. One carrying nothing else rides quietly rather than firing — or,
    // before this rule, reading numbers a turret need not declare.
    let mut app = utils::transport_app();
    let world = app.world_mut();
    let (bunker, bunker_id) =
        utils::create_entity(world, "bunker", utils::pos(10, 10), Some(0)).unwrap();
    let (rider, rider_id) =
        utils::create_entity(world, "gun_rider", utils::pos(9, 10), Some(0)).unwrap();
    // In the gun's reach from the bunker AND from the rider's own boarding cell,
    // so a shot wrongly fired from either place would land.
    let (foe, _) = utils::create_entity(world, "civilian", utils::pos(12, 10), Some(2)).unwrap();

    utils::select(&mut app, rider_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Board {
            target: bunker_id,
            flush: true,
        },
    );
    utils::run_until_aboard(&mut app, bunker, 1, 40);
    utils::run_ticks(&mut app, 20);

    assert!(
        utils::passengers_of(app.world(), bunker).contains(&rider_id),
        "the rider is aboard"
    );
    assert!(entity_def::hidden(app.world(), rider), "and stowed");
    assert_eq!(
        utils::health_as_u32(&app, foe),
        20,
        "its gun stayed silent: a turret has no place to stand from inside"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Runs ticks until `entity` is off the map, at most `limit` of them.
fn run_until_hidden(app: &mut App, entity: Entity, limit: u32) {
    for _ in 0..limit {
        if entity_def::hidden(app.world(), entity) {
            return;
        }
        utils::run_ticks(app, 1);
    }
    assert!(
        entity_def::hidden(app.world(), entity),
        "the entity went off the map within {limit} ticks"
    );
}

/// Sends a rifleman of player 0 walking south from (11, 10) while `shooter`
/// fires a following shot at it, and boards it into `wagon` on (11, 12)
/// before the shot lands.
fn board_walker_under_fire(app: &mut App, shooter: SimulationId, wagon: Entity) -> Entity {
    let (rifleman, rifleman_id) = utils::create_owned(app, "rifleman", 11, 10, 0);
    utils::select(app, rifleman_id);
    utils::push_command(
        app,
        PlayerCommand::Move {
            target: utils::pos(11, 19),
            flush: true,
        },
    );
    utils::attack(app, shooter, rifleman_id);
    // The shot leaves on tick 3, sent to (11, 10) where the walker then is,
    // and lands on tick 11; by tick 6 the walker stands on (11, 12) and
    // climbs aboard there.
    utils::run_ticks(app, 6);
    assert_eq!(
        spawn::board(app.world_mut(), rifleman, wagon),
        Boarding::Boarded
    );
    let went_in_at = app
        .world()
        .get::<HiddenComponent>(rifleman)
        .expect("a passenger is hidden")
        .went_in
        .position;
    assert_eq!(went_in_at, utils::pos(11, 12));
    rifleman
}
