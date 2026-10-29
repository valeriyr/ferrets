//! Warp-in: a warper sets its unit down rising at once on a picked cell.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    build::BuilderAttendance,
    detection::Detection,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldCoverage, FieldDecay, FieldDef, FieldGrowth, FieldPlacement, FieldSourceDef,
        FieldVision,
    },
    landing::Landing,
    location::Solidity,
    pool::Pool,
    pool_def::PoolId,
    price,
    registry::ContentRegistry,
    requirement::Requirement,
    resource::DepletionPolicy,
    terrains::Terrains,
    work::{CrewLimit, WorkPresence},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    beside::Lean,
    command::PlayerCommand,
    components::{
        morph::MorphComponent, rally::RallyPointComponent, resource::ResourceSourceComponent,
        train::TrainQueueComponent, warp::WarpComponent,
    },
    entity_def,
    events::{DeathCause, SimulationEvent, SpendCause},
    game_loop::{
        orders::Refusal,
        warp::{self, WarpRefusal},
    },
    order::Order,
    resources::PlayerResources,
    session::GameSession,
    spawn,
};

//
// ─── Warping in ──────────────────────────────────────────────────────────────
//

#[test]
fn relay_warps_blade_in_rising_at_picked_cell() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);

    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    assert_eq!(utils::cell_of(app.world(), blade), CellPos::new(12, 10));
    assert!(entity_def::rising(app.world(), blade));
    // 100 gold granted, 50 paid.
    assert_eq!(utils::gold(app.world()), 50);

    utils::run_ticks(&mut app, RISE_TIME + 1);
    assert!(!entity_def::rising(app.world(), blade));
}

#[test]
fn relay_refuses_cell_outside_its_power() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // Four cells east of the beacon, past its power radius of three.
    assert_eq!(judged(&app, relay, 14, 10), Err(WarpRefusal::OutsideField));
}

#[test]
fn relay_refuses_cell_ground_does_not_take() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // The beacon stands there.
    assert_eq!(judged(&app, relay, 10, 10), Err(WarpRefusal::Blocked));
}

#[test]
fn rising_blade_holds_its_supply() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (other, _) = utils::create_owned(&mut app, "relay", 5, 12, 0);
    utils::run_ticks(&mut app, 1);
    utils::grant_gold(&mut app, 100);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);

    // The beacon provides one; the rising blade holds it.
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    assert!(entity_def::rising(app.world(), blade));
    assert_eq!(judged(&app, other, 12, 12), Err(WarpRefusal::NoSupply));
}

#[test]
fn warper_queues_nothing_for_train_command() {
    let mut app = app();
    // Supply for the blade, so only the relay training nothing refuses the
    // command.
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);

    utils::train_entity(&mut app, relay_id, "blade");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert!(app.world().get::<TrainQueueComponent>(relay).is_none());
    // Nothing paid.
    assert_eq!(utils::gold(app.world()), 100);
}

#[test]
fn canceling_warp_refunds_blade_and_resets_relay() {
    let mut app = app();
    utils::record_announcements(&mut app);
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    let blade_type = entity_def::type_id(app.world(), blade);
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: blade_id });
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back, both as the relay's warp.
    assert_eq!(utils::gold(app.world()), 100);
    let announced = &app.world().resource::<utils::Announced>().0;
    let warp = SpendCause::Warp { warper: relay_id };
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::ResourcesSpent { cause, .. } if *cause == warp
    )));
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::ResourcesRefunded { cause, .. } if *cause == warp
    )));
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::EntityDied { entity, cause: DeathCause::Canceled, .. } if *entity == blade_id
    )));
    // Its supply freed and the relay's cooldown reset: a second warp-in goes.
    assert_eq!(judged(&app, relay, 12, 12), Ok(()));
    // Called off, it is never counted as produced.
    utils::run_ticks(&mut app, RISE_TIME + 1);
    assert_eq!(utils::produced(&app, blade_type), 0);
}

#[test]
fn blade_losing_its_power_is_called_off_and_refunded() {
    let mut app = app();
    // Two beacons, each powering three cells about itself, far enough apart
    // that the blade stands in the near one's power alone.
    let (near, _) = utils::create_owned(&mut app, "beacon", 10, 10, 0);
    utils::create_owned(&mut app, "beacon", 20, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    spawn::despawn_entity(app.world_mut(), near, DeathCause::Decayed);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back.
    assert_eq!(utils::gold(app.world()), 100);
    // The relay's cooldown reset: it warps in again on the other beacon's
    // power at once.
    assert_eq!(judged(&app, relay, 21, 10), Ok(()));
}

#[test]
fn builder_sent_to_blade_warping_in_follows_rather_than_builds() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 12, 13, 0);
    utils::run_ticks(&mut app, 1);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    utils::send_to(&mut app, mason_id, blade_id);
    utils::run_ticks(&mut app, utils::APPLY);

    // A unit warping in is no site to help raise, though the mason builds
    // its type: the click falls through to following it, walking up first.
    let orders = entity_def::orders(app.world(), mason);
    assert!(
        matches!(orders.last(), Some(Order::Follow { target }) if *target == blade_id),
        "the mason follows the blade, given {orders:?}"
    );
}

#[test]
fn calling_off_older_warp_leaves_later_warps_cooldown() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    utils::create_owned(&mut app, "beacon", 10, 14, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // Lancer A, and lancer B once the relay has cooled down from A, while A
    // still rises.
    utils::warp_in(&mut app, relay_id, "lancer", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let first = utils::single_owned_of_type(app.world_mut(), "lancer", 0);
    let first_id = entity_def::simulation_id(app.world(), first);
    utils::run_ticks(&mut app, LANCER_TRAIN_TIME);
    utils::warp_in(&mut app, relay_id, "lancer", 12, 12);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "lancer"), 2);

    // Calling A off gives A's price back but not B's cooldown.
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: first_id });
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(!utils::alive(&app, first_id));
    assert_eq!(
        warp::judge(app.world(), 0, relay, "lancer", utils::pos(14, 10)).map(|_| ()),
        Err(WarpRefusal::CoolingDown)
    );
}

#[test]
fn warp_called_off_after_relay_changed_hands_leaves_relays_cooldown() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (_, taker) = utils::create_owned(&mut app, "relay", 20, 20, 1);
    utils::run_ticks(&mut app, 1);
    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    let cooling = warp_ready_at(&app, relay);

    // The relay is player 1's now: calling player 0's warp off refunds player
    // 0 and leaves the relay's cooldown alone.
    spawn::change_owner(app.world_mut(), relay, 1, taker, Lean::Nothing);
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: blade_id });
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back.
    assert_eq!(utils::gold(app.world()), 100);
    assert_eq!(warp_ready_at(&app, relay), cooling);
}

#[test]
fn captured_warp_losing_its_field_refunds_its_payer() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (_, taker) = utils::create_owned(&mut app, "relay", 20, 20, 1);
    utils::run_ticks(&mut app, 1);
    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);

    // Handed to player 1, whose power does not reach it: the warp is called
    // off, its price going back to player 0, who paid it.
    spawn::change_owner(app.world_mut(), blade, 1, taker, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back.
    assert_eq!(utils::gold(app.world()), 100);
    assert_eq!(
        app.world().resource::<PlayerResources>().amount(1, "gold"),
        0
    );
    // The relay still player 0's: its cooldown from the warp is reset.
    assert_eq!(judged(&app, relay, 12, 12), Ok(()));
}

#[test]
fn relay_refuses_type_outside_its_catalogue_and_non_warper_refuses_all() {
    let mut app = app();
    let (beacon, _) = utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (watch_relay, _) = utils::create_owned(&mut app, "watch_relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // A watch relay warps blades in, not lancers; a beacon warps nothing in.
    assert_eq!(
        judged_type(&app, watch_relay, "lancer", 12, 10),
        Err(WarpRefusal::Incapable)
    );
    assert_eq!(judged(&app, beacon, 12, 10), Err(WarpRefusal::Incapable));
}

#[test]
fn relay_changing_form_refuses_warp() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    let from = entity_def::type_id(app.world(), relay);
    app.world_mut()
        .entity_mut(relay)
        .insert(MorphComponent::under_way(from, "relay".into(), Vec::new()));

    assert_eq!(judged(&app, relay, 12, 10), Err(WarpRefusal::Busy));
}

#[test]
fn relay_warps_drill_in_over_vein_covering_it_and_refuses_open_ground() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (vein, vein_id) = utils::create_entity(app.world_mut(), "vein", utils::pos(12, 10), None)
        .expect("the vein is registered");
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(vein)
        .expect("a vein is a resource source")
        .amount = 40;
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        judged_type(&app, relay, "drill", 12, 12),
        Err(WarpRefusal::Blocked)
    );

    utils::record_announcements(&mut app);
    utils::warp_in(&mut app, relay_id, "drill", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let drill = utils::single_owned_of_type(app.world_mut(), "drill", 0);
    assert!(entity_def::rising(app.world(), drill));
    assert_eq!(utils::cell_of(app.world(), drill), CellPos::new(12, 10));
    // The vein is covered, not merely gone: it dies as overbuilt and the drill
    // holds the 40 it had left.
    let announced = &app.world().resource::<utils::Announced>().0;
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::EntityDied { entity, cause: DeathCause::Overbuilt, .. }
            if *entity == vein_id
    )));
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(drill)
            .expect("a drill is a resource source")
            .amount,
        40
    );
}

#[test]
fn relay_refuses_well_over_spring_walker_stands_on_until_walker_dies() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::create_entity(app.world_mut(), "spring", utils::pos(12, 10), None)
        .expect("the spring is registered");
    // The spring claims no cell, so a walker stands on it.
    let (lookout, lookout_id) = utils::create_owned(&mut app, "lookout", 12, 10, 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        judged_type(&app, relay, "well", 12, 10),
        Err(WarpRefusal::Blocked)
    );

    // Warped in all the same, it costs nothing and raises nothing.
    utils::warp_in(&mut app, relay_id, "well", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "well"), 0);
    // The 100 gold granted, all kept.
    assert_eq!(utils::gold(app.world()), 100);

    spawn::despawn_entity(app.world_mut(), lookout, DeathCause::Decayed);
    utils::run_ticks(&mut app, 1);
    assert!(!utils::alive(&app, lookout_id));
    assert_eq!(judged_type(&app, relay, "well", 12, 10), Ok(()));
}

#[test]
fn rising_relay_refuses_warp() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_unattended_rising(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        judged(&app, relay, 12, 10),
        Err(WarpRefusal::Warper(Refusal::Rising))
    );
}

#[test]
fn relay_refuses_unit_its_requirements_bar_until_met() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        judged_type(&app, relay, "elite", 12, 10),
        Err(WarpRefusal::Unmet)
    );

    utils::create_owned(&mut app, "shrine", 3, 3, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(judged_type(&app, relay, "elite", 12, 10), Ok(()));
}

#[test]
fn relay_refuses_unit_its_owner_cannot_pay() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // A knight costs 150 of the 100 granted: refused, and the command takes
    // nothing.
    assert_eq!(
        judged_type(&app, relay, "knight", 12, 10),
        Err(WarpRefusal::Unaffordable)
    );
    utils::warp_in(&mut app, relay_id, "knight", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 0);
    // The 100 granted, untouched.
    assert_eq!(utils::gold(app.world()), 100);
}

#[test]
fn sighted_relay_refuses_cell_its_side_does_not_see() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, _) = utils::create_owned(&mut app, "watch_relay", 5, 10, 0);
    // The lookout sees two cells about itself: (12, 9) lies in its sight,
    // (12, 13) beyond it, both in the beacon's power.
    utils::create_owned(&mut app, "lookout", 12, 8, 0);
    utils::run_ticks(&mut app, 2);

    assert_eq!(judged(&app, relay, 12, 13), Err(WarpRefusal::Unseen));
    assert_eq!(judged(&app, relay, 12, 9), Ok(()));
}

#[test]
fn relay_refuses_cell_unit_itself_may_not_stand_on() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    utils::create_owned(&mut app, "puddle", 11, 11, 0);
    let (relay, _) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    // The puddle muds the four cells beside it, inside the beacon's power: a
    // sapper, which may not stand on mud, is refused at (12, 11) and admitted
    // at (12, 10), a step off the mud.
    assert_eq!(
        judged_type(&app, relay, "sapper", 12, 11),
        Err(WarpRefusal::OutsideField)
    );
    assert_eq!(judged_type(&app, relay, "sapper", 12, 10), Ok(()));
}

#[test]
fn relay_cools_down_exactly_its_units_train_time() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    utils::create_owned(&mut app, "beacon", 10, 14, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::grant_gold(&mut app, 100);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    // The warp's tick put in the blade's first tick of rise, so the session
    // stands `progress` ticks past it.
    let warped_at = app.world().resource::<GameSession>().tick()
        - utils::rise_progress(&app, blade).expect("the blade still rises");
    // Ready again at the warp's tick + 30, not a tick sooner.
    while app.world().resource::<GameSession>().tick() + 1 < warped_at + TRAIN_TIME {
        utils::run_ticks(&mut app, 1);
    }
    assert_eq!(judged(&app, relay, 12, 12), Err(WarpRefusal::CoolingDown));
    utils::run_ticks(&mut app, 1);
    assert_eq!(judged(&app, relay, 12, 12), Ok(()));
}

#[test]
fn blade_losing_power_on_its_last_tick_of_rise_is_called_off_not_risen() {
    let mut app = app();
    let (beacon, _) = utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    // One tick of rise left: the field goes before it is put in.
    while utils::rise_progress(&app, blade).expect("the blade still rises") + 1 < RISE_TIME {
        utils::run_ticks(&mut app, 1);
    }
    spawn::despawn_entity(app.world_mut(), beacon, DeathCause::Decayed);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back.
    assert_eq!(utils::gold(app.world()), 100);
}

#[test]
fn blade_whose_relay_died_is_refunded_when_its_power_goes() {
    let mut app = app();
    let (beacon, _) = utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);

    // The relay dies: the blade goes on rising; then the beacon goes, and
    // the warp is called off with no relay left to reset.
    spawn::despawn_entity(app.world_mut(), relay, DeathCause::Decayed);
    utils::run_ticks(&mut app, 2);
    assert!(utils::alive(&app, blade_id));
    spawn::despawn_entity(app.world_mut(), beacon, DeathCause::Decayed);
    utils::run_ticks(&mut app, 1);

    assert!(!utils::alive(&app, blade_id));
    // 100 granted, 50 paid and 50 paid back.
    assert_eq!(utils::gold(app.world()), 100);
}

#[test]
fn hall_both_trains_and_warps_blades() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    utils::create_owned(&mut app, "beacon", 10, 14, 0);
    let (hall, hall_id) = utils::create_owned(&mut app, "hall", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::train_entity(&mut app, hall_id, "blade");
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::train_queue_len(app.world(), hall), 1);
    // The queue does not refuse the warp, nor the warp the queue: 100 paid
    // for both.
    utils::warp_in(&mut app, hall_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::gold(app.world()), 0);
    utils::run_ticks(&mut app, TRAIN_TIME + RISE_TIME);
    assert_eq!(utils::count_of_type(app.world_mut(), "blade"), 2);
}

//
// ─── Rally ───────────────────────────────────────────────────────────────────
//

#[test]
fn warped_blade_takes_rally_set_on_it_while_rising() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::set_rally(&mut app, relay_id, 20, 20);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    let blade_id = entity_def::simulation_id(app.world(), blade);
    utils::set_rally(&mut app, blade_id, 25, 5);
    utils::run_ticks(&mut app, RISE_TIME + 1);

    // Its own rally outranks the relay's.
    assert_eq!(utils::move_target(&app, blade), Some(utils::pos(25, 5)));
    // Spent once risen: a blade releases nothing and keeps no rally point.
    assert!(app.world().get::<RallyPointComponent>(blade).is_none());
}

#[test]
fn warped_blade_without_rally_of_its_own_takes_relays_current_one() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::set_rally(&mut app, relay_id, 20, 20);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    assert!(entity_def::orders(app.world(), blade).is_empty());
    // The relay's rally moves while the blade rises; the one standing when it
    // has risen is the one it takes.
    utils::set_rally(&mut app, relay_id, 22, 22);
    utils::run_ticks(&mut app, RISE_TIME + 1);

    assert_eq!(utils::move_target(&app, blade), Some(utils::pos(22, 22)));
}

#[test]
fn warped_blade_with_no_rally_stays_where_set_down() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + RISE_TIME + 1);

    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    assert!(!entity_def::rising(app.world(), blade));
    assert!(entity_def::orders(app.world(), blade).is_empty());
    assert_eq!(utils::cell_of(app.world(), blade), CellPos::new(12, 10));
}

#[test]
fn warped_blade_is_sent_to_its_rally_tick_it_rises() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::set_rally(&mut app, relay_id, 20, 5);
    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    while utils::rise_progress(&app, blade).expect("the blade still rises") + 1 < RISE_TIME {
        utils::run_ticks(&mut app, 1);
    }

    // The tick that puts in its last tick of rise also sends it on.
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::rising(app.world(), blade));
    assert_eq!(utils::move_target(&app, blade), Some(utils::pos(20, 5)));
}

#[test]
fn warped_blade_takes_no_rally_from_relay_since_captured() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    let (_, taker) = utils::create_owned(&mut app, "relay", 20, 20, 1);
    utils::run_ticks(&mut app, 1);
    utils::set_rally(&mut app, relay_id, 20, 5);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    // The relay changes hands while the blade rises: its rally is another
    // player's now, and the risen blade follows none.
    spawn::change_owner(app.world_mut(), relay, 1, taker, Lean::Nothing);
    utils::run_ticks(&mut app, RISE_TIME + 1);

    assert!(!entity_def::rising(app.world(), blade));
    assert!(entity_def::orders(app.world(), blade).is_empty());
}

#[test]
fn warped_blade_whose_relay_died_stays_where_set_down() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (relay, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    utils::set_rally(&mut app, relay_id, 20, 5);

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    let blade = utils::single_owned_of_type(app.world_mut(), "blade", 0);
    // The relay dying does not stop the warp, and leaves no rally to follow.
    spawn::despawn_entity(app.world_mut(), relay, DeathCause::Decayed);
    utils::run_ticks(&mut app, RISE_TIME + 1);

    assert!(!entity_def::rising(app.world(), blade));
    assert!(entity_def::orders(app.world(), blade).is_empty());
    assert_eq!(utils::cell_of(app.world(), blade), CellPos::new(12, 10));
}

//
// ─── Production ──────────────────────────────────────────────────────────────
//

#[test]
fn warped_blade_counts_toward_production_once_risen() {
    let mut app = app();
    utils::create_owned(&mut app, "beacon", 10, 10, 0);
    let (_, relay_id) = utils::create_owned(&mut app, "relay", 5, 10, 0);
    utils::run_ticks(&mut app, 1);
    let blade = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("blade")
        .expect("the fixture registers a blade");

    utils::warp_in(&mut app, relay_id, "blade", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::produced(&app, blade), 0);

    utils::run_ticks(&mut app, RISE_TIME + 1);
    assert_eq!(utils::produced(&app, blade), 1);
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Ticks a `blade` rises over.
const RISE_TIME: u32 = 10;

/// Ticks a `relay` cools down for after warping in a `blade`.
const TRAIN_TIME: u32 = 30;

/// Ticks a `relay` cools down for after warping in a `lancer`.
const LANCER_TRAIN_TIME: u32 = 20;

/// Ticks a `lancer` rises over.
const LANCER_RISE_TIME: u32 = 60;

/// App with two players, player 0 holding 100 gold, and the warp roster: a
/// `power` field its `beacon`s spread, the warpers and the types they warp
/// in, each described where it is registered.
fn app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        let power = registry.register_field(
            "power",
            FieldDef::new(
                Terrains::Only(utils::LAND.into()),
                utils::GROUND.into(),
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            utils::building("beacon")
                .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::ONE)
                .with_field_sources([FieldSourceDef::new(
                    power,
                    3,
                    FieldGrowth::Instant,
                    Emission::Nothing,
                    Emission::Nothing,
                )]),
        );
        let powered = || {
            vec![FieldPlacement::Requires {
                field: power,
                of: Affiliation::Own,
                coverage: FieldCoverage::Every,
            }]
        };
        registry.register(
            utils::building("relay")
                .with_rise(5, utils::rise_initial_pools(&[PoolId::HEALTH]))
                .with_warper(
                    [
                        "blade", "lancer", "elite", "knight", "sapper", "drill", "well",
                    ],
                    powered(),
                    Landing::Anywhere,
                ),
        );
        // A relay that warps only onto cells its side sees, and a hall that
        // both trains and warps blades.
        registry.register(utils::building("watch_relay").with_warper(
            ["blade"],
            powered(),
            Landing::Sighted,
        ));
        registry.register(utils::building("hall").with_trainer(["blade"]).with_warper(
            ["blade"],
            powered(),
            Landing::Anywhere,
        ));
        registry.register(blade_like("blade", 50));
        // An elite needs a shrine standing; a knight costs more than the 100
        // gold granted; a sapper may not stand on mud.
        registry.register(utils::building("shrine"));
        registry.register(
            blade_like("elite", 50).with_requires([Requirement::EntityType("shrine".into())]),
        );
        registry.register(blade_like("knight", 150));
        let mud = registry.register_field(
            "mud",
            FieldDef::new(
                Terrains::Only(utils::LAND.into()),
                utils::GROUND.into(),
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            utils::building("puddle").with_field_sources([FieldSourceDef::new(
                mud,
                1,
                FieldGrowth::Instant,
                Emission::Nothing,
                Emission::Nothing,
            )]),
        );
        registry.register(
            blade_like("sapper", 50).with_field_placement([FieldPlacement::Forbids { field: mud }]),
        );
        // A drill is raised over a vein, a source of gold, which it covers.
        registry.register(
            utils::building("vein").with_resource_source("gold", DepletionPolicy::Destroy),
        );
        // A spring claims no cell; a well is raised over one.
        registry.register(
            EntityTypeDef::new("spring")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Passable,
                    utils::GROUND,
                )
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            utils::building("well")
                .with_price(price::from([("gold", 50)]))
                .with_train_time(TRAIN_TIME)
                .with_rise(RISE_TIME, utils::rise_initial_pools(&[PoolId::HEALTH]))
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("spring"),
        );
        registry.register(
            utils::building("drill")
                .with_price(price::from([("gold", 50)]))
                .with_train_time(TRAIN_TIME)
                .with_rise(RISE_TIME, utils::rise_initial_pools(&[PoolId::HEALTH]))
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("vein"),
        );
        // A lookout sees two cells about itself.
        registry.register(
            utils::walker("lookout", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_sight_range(2),
        );
        // A lancer rises for longer than its warper cools down for it.
        registry.register(
            utils::walker("lancer", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_price(price::from([("gold", 50)]))
                .with_train_time(LANCER_TRAIN_TIME)
                .with_rise(
                    LANCER_RISE_TIME,
                    utils::rise_initial_pools(&[PoolId::HEALTH]),
                )
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE),
        );
        registry.register(
            utils::walker("mason", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["blade"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    utils::grant_gold(&mut app, 100);
    app
}

/// A one-cell walker named `name` like a blade: costing `gold` and one supply,
/// trained in `TRAIN_TIME` and rising over `RISE_TIME`.
fn blade_like(name: &str, gold: u32) -> EntityTypeDef {
    utils::walker(name, utils::GROUND_DOMAIN)
        .with_pool(Pool::health(20))
        .with_dying(1, [])
        .with_price(price::from([("gold", gold)]))
        .with_train_time(TRAIN_TIME)
        .with_rise(RISE_TIME, utils::rise_initial_pools(&[PoolId::HEALTH]))
        .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE)
}

/// What judging a `blade` warped in by `relay` at `(x, y)` answers now.
fn judged(app: &App, relay: Entity, x: u32, y: u32) -> Result<(), WarpRefusal> {
    judged_type(app, relay, "blade", x, y)
}

/// What judging a `type_name` warped in by `warper` at `(x, y)` answers now.
fn judged_type(
    app: &App,
    warper: Entity,
    type_name: &str,
    x: u32,
    y: u32,
) -> Result<(), WarpRefusal> {
    warp::judge(app.world(), 0, warper, type_name, utils::pos(x, y)).map(|_| ())
}

/// The tick `warper`'s cooldown runs out at.
fn warp_ready_at(app: &App, warper: Entity) -> u32 {
    app.world()
        .get::<WarpComponent>(warper)
        .expect("a warper keeps its cooldown")
        .ready_at
}
