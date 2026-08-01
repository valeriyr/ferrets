//! Repair order: menders restoring damaged entities.

mod utils;

use std::collections::BTreeSet;

use bevy::prelude::*;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::{astar, nav_pos::NavPos, nav_size::NavSize};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        build::UnderConstructionComponent, energy::EnergyComponent, entity_stats::StatsComponent,
        hidden::HiddenComponent, location::LocationComponent, repair::UnderRepairComponent,
    },
    content::{
        entity_stats::EntityStatId,
        entity_type_def::EntityTypeDef,
        location::Solidity,
        registry::ContentRegistry,
        repair::{RepairCost, RepairRate},
        work::WorkPresence,
    },
    resources,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn,
};

//
// ─── Mending ────────────────────────────────────────────────────────────────
//

#[test]
fn repair_restores_health_at_target_production_rate() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    // A 100-point pool over a build_time of 20 at speed 1 mends 5 points a tick, so
    // the 40 points lost need eight ticks of work once the two-tick walk is done.
    utils::run_ticks(&mut app, utils::APPLY + 2 + 4);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(80),
        "four ticks of work land exactly four times the per-tick amount"
    );

    utils::run_ticks(&mut app, 4);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the depot is mended back to its full pool on the eighth tick of work"
    );
    utils::run_ticks(&mut app, 1);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), worker),
        "the order ends once there is nothing left to mend"
    );
}

#[test]
fn repair_ratio_scales_work_against_production_time() {
    let mut app = app();
    // Same pool and build_time as the depot, but declared to mend in half the time.
    let (hall, hall_id) = utils::spawn_owned(&mut app, "hall", 10, 10, 0);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, hall, 50.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, hall_id);
    // Ten points a tick rather than the depot's five, so five ticks of work.
    utils::run_ticks(&mut app, utils::APPLY + 3 + 5);

    assert_eq!(
        utils::current_health(&app, hall),
        FixedU64::from_num(100),
        "halving the ratio doubles the rate"
    );
}

#[test]
fn several_workers_mend_faster_than_one() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (_, first) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    let (_, second) = utils::spawn_owned(&mut app, "worker", 8, 11, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, first, depot_id);
    repair(&mut app, second, depot_id);
    // Four ticks of work between them instead of eight.
    utils::run_ticks(&mut app, utils::APPLY + 3 + 4);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "two workers on a stacking job each contribute their own rate"
    );
}

//
// ─── A flat rate, paid out of the worker ────────────────────────────────────
//

#[test]
fn flat_rate_ignores_what_target_cost_to_produce() {
    // The depot is built in 20 ticks, the hall mends in 10, and a flat-rate mender
    // must not care about either: both take the same eight ticks for 40 points.
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (hall, hall_id) = utils::spawn_owned(&mut app, "hall", 20, 20, 0);
    let (_, first) = utils::spawn_owned(&mut app, "medic", 8, 10, 0);
    let (_, second) = utils::spawn_owned(&mut app, "medic", 18, 20, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::wound(&mut app, hall, 40.0);

    // Both medics already reach their patient from where they stand, so the two
    // jobs start together and can be compared tick for tick.
    repair(&mut app, first, depot_id);
    repair(&mut app, second, hall_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);

    let partway = utils::current_health(&app, depot);
    assert!(
        partway > FixedU64::from_num(60) && partway < FixedU64::from_num(100),
        "the depot's job is under way but not done, at {partway}"
    );
    assert_eq!(
        utils::current_health(&app, hall),
        partway,
        "the hall keeps exact pace, its faster repair_ratio notwithstanding"
    );

    utils::run_ticks(&mut app, 4);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "a flat five points a tick fills the depot in eight ticks"
    );
    assert_eq!(
        utils::current_health(&app, hall),
        FixedU64::from_num(100),
        "and fills the hall on the same tick"
    );
}

#[test]
fn flat_rate_mends_target_nothing_produces() {
    let mut app = app();
    // No build_time and no train_time — unmendable at a production-paced rate.
    let (monolith, monolith_id) = utils::spawn_owned(&mut app, "monolith", 10, 10, 0);
    let (_, medic_id) = utils::spawn_owned(&mut app, "medic", 8, 10, 0);
    utils::wound(&mut app, monolith, 40.0);

    repair(&mut app, medic_id, monolith_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, monolith),
        FixedU64::from_num(100),
        "a flat rate needs no production time to pace itself against"
    );
}

#[test]
fn repair_range_lets_mender_work_without_closing_in() {
    let mut app = app();
    // The depot's footprint starts at (10, 10); the medic stands two cells short of
    // it, which its repair_range of 2 already covers.
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (medic, medic_id) = utils::spawn_owned(&mut app, "medic", 8, 10, 0);
    utils::wound(&mut app, depot, 20.0);
    let stood_at = utils::cell_of(app.world_mut(), medic);

    repair(&mut app, medic_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 10);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the work lands from where it stood"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), medic),
        stood_at,
        "a longer reach means no step toward the patient at all"
    );
}

#[test]
fn energy_paid_repair_spends_worker_pool_not_treasury() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (medic, medic_id) = utils::spawn_owned(&mut app, "medic", 8, 10, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, medic_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the depot is mended"
    );
    assert_eq!(
        utils::gold(app.world()),
        500,
        "not a coin of the owner's gold went into it"
    );
    // Half a point of energy per point of health, over 40 points restored.
    assert_eq!(
        energy(&app, medic),
        FixedU64::from_num(30),
        "the work came out of the worker's own pool"
    );
}

#[test]
fn spent_medic_waits_at_patient_and_resumes_once_it_can_pay() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (medic, medic_id) = utils::spawn_owned(&mut app, "medic", 8, 10, 0);
    // More damage than the energy left can pay for: 10 energy buys 20 health.
    utils::wound(&mut app, depot, 90.0);
    drain_energy(&mut app, medic, 40.0);

    repair(&mut app, medic_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(30),
        "the medic spends its last energy and stops there"
    );
    assert_eq!(
        energy(&app, medic),
        FixedU64::ZERO,
        "the pool is empty rather than overdrawn"
    );
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), medic),
        "with no patience limit it stays at the patient instead of giving up"
    );

    grant_energy(&mut app, medic, 50.0);
    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the held job resumes the moment the work can be paid for"
    );
}

//
// ─── Cost ───────────────────────────────────────────────────────────────────
//

#[test]
fn full_repair_bills_cost_factor_share_of_price() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    // A whole pool restored at a factor of 0.5 on a 200-gold depot bills 100.
    utils::wound(&mut app, depot, 100.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the depot is fully mended"
    );
    assert_eq!(
        utils::gold(app.world()),
        400,
        "mending the whole pool costs half the depot's price, charged in fractions \
         that add up exactly"
    );
}

#[test]
fn workers_on_one_job_split_bill_rather_than_each_paying_it() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (_, first) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    let (_, second) = utils::spawn_owned(&mut app, "worker", 8, 11, 0);
    utils::wound(&mut app, depot, 100.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, first, depot_id);
    repair(&mut app, second, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(
        utils::gold(app.world()),
        400,
        "a crew buys speed, not a discount: the bill follows the health restored"
    );
}

#[test]
fn flat_per_tick_cost_is_charged_for_each_worker() {
    // Forty points at five a tick is eight worker-ticks of work however it is
    // divided, and a flat rate bills each of them once.
    let one = per_tick_spend(&["hauler"]);
    let two = per_tick_spend(&["hauler", "hauler"]);

    assert_eq!(one, 8, "one worker pays for the eight ticks it works");
    assert_eq!(
        two, one,
        "charging per worker keeps the bill the same when a second one joins — the \
         crew buys speed, not a discount"
    );
}

#[test]
fn unaffordable_repair_holds_job_then_abandons_it() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, depot, 100.0);
    // Enough for part of the work, then nothing.
    utils::grant_gold(&mut app, 10);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 3 + 6);

    let stalled_at = utils::current_health(&app, depot);
    assert!(
        stalled_at > FixedU64::from_num(0) && stalled_at < FixedU64::from_num(100),
        "work stops partway once the gold runs out, at {stalled_at}"
    );
    assert_eq!(utils::gold(app.world()), 0, "every coin went into the work");
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), worker),
        "the worker waits at the job while it cannot pay"
    );

    // Patience is 5 ticks, and it has already stalled for several.
    utils::run_ticks(&mut app, 10);
    assert_eq!(
        utils::current_health(&app, depot),
        stalled_at,
        "no work lands while the owner is broke"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), worker),
        "the job is abandoned once patience runs out"
    );
}

#[test]
fn patient_repairer_waits_indefinitely_and_resumes_when_paid() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "stoic", 8, 10, 0);
    utils::wound(&mut app, depot, 100.0);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 30);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(0),
        "nothing is mended without the gold to pay for it"
    );
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), worker),
        "a repairer with no patience limit never gives up on the job"
    );

    utils::grant_gold(&mut app, 500);
    utils::run_ticks(&mut app, 30);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the held job resumes as soon as the work can be paid for"
    );
}

//
// ─── Who mends what ─────────────────────────────────────────────────────────
//

#[test]
fn repairer_refuses_target_without_tag_it_mends() {
    let mut app = app();
    // A damageable, produced unit — but not tagged as a building.
    let (soldier, soldier_id) = utils::spawn_owned(&mut app, "soldier", 10, 10, 0);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, soldier, 10.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, soldier_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, soldier),
        FixedU64::from_num(10),
        "a worker that only mends buildings leaves a wounded soldier alone"
    );
}

#[test]
fn repairer_refuses_target_nothing_produces() {
    let mut app = app();
    // Tagged as a building, but with no build_time to pace the work against.
    let (monolith, monolith_id) = utils::spawn_owned(&mut app, "monolith", 10, 10, 0);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, monolith, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, monolith_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, monolith),
        FixedU64::from_num(60),
        "repair paces itself against production, so an unproduced type has no rate"
    );
}

#[test]
fn repairer_refuses_enemy_target() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 1);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(60),
        "an enemy structure is never mended"
    );
}

#[test]
fn repairer_refuses_target_still_under_construction() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (_, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, depot, 40.0);
    app.world_mut()
        .entity_mut(depot)
        .insert(UnderConstructionComponent::default());
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(60),
        "an unfinished site is the build order's business, not repair's"
    );
}

#[test]
fn self_repair_is_refused_unless_declared() {
    let mut app = app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    utils::wound(&mut app, worker, 10.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, worker_id);
    utils::run_ticks(&mut app, utils::APPLY + 20);

    assert_eq!(
        utils::current_health(&app, worker),
        FixedU64::from_num(10),
        "a worker that has not opted into self-repair cannot mend itself"
    );
}

//
// ─── Sharing a job ──────────────────────────────────────────────────────────
//

#[test]
fn exclusive_job_turns_second_worker_away() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (_, first) = utils::spawn_owned(&mut app, "loner", 8, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "loner", 8, 11, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, first, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    repair(&mut app, second_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(
        utils::order_queue_is_empty(app.world_mut(), second),
        "a worker that does not share a job refuses one already taken"
    );
}

#[test]
fn mended_target_records_crew_until_last_worker_leaves() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (first, first_id) = utils::spawn_owned(&mut app, "worker", 8, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "worker", 8, 11, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    assert!(
        crew_of(&app, depot).is_none(),
        "an intact depot has no crew"
    );

    repair(&mut app, first_id, depot_id);
    repair(&mut app, second_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        crew_of(&app, depot),
        Some(BTreeSet::from([first_id, second_id])),
        "both menders show up in the job's crew"
    );

    // One of the pair stops. The other is still mending, so the job keeps its crew.
    utils::stop_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        crew_of(&app, depot),
        Some(BTreeSet::from([second_id])),
        "one worker leaving does not free the job"
    );

    // The mark goes with the last worker off the job.
    utils::stop_orders(app.world_mut(), second);
    utils::run_ticks(&mut app, 1);
    assert!(crew_of(&app, depot).is_none());
}

#[test]
fn hidden_worker_leaves_map_and_comes_back() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "mole", 8, 10, 0);
    utils::wound(&mut app, depot, 20.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(
        app.world().get::<HiddenComponent>(worker).is_some(),
        "a worker that works from inside its job is taken off the map"
    );

    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the work still lands while the worker is out of sight"
    );
    assert!(
        app.world().get::<HiddenComponent>(worker).is_none(),
        "the worker comes back out once the job is done"
    );
}

#[test]
fn boxed_in_hidden_worker_finishes_job_and_waits_to_reappear() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "mole", 8, 10, 0);
    utils::wound(&mut app, depot, 20.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(app.world().get::<HiddenComponent>(worker).is_some());

    // Take away every cell it could come back out onto, then let the work finish.
    utils::set_all_cells_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 10);

    // The mending is not held back by having nowhere to put the worker: the job
    // finishes and the worker waits off the map with a queued reveal, coming back
    // onto the one cell that frees.
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the job finished regardless"
    );
    utils::assert_reveal_deferred_then_lands_on(&mut app, worker, NavPos::new(9, 9));
}

#[test]
fn cancel_brings_hidden_worker_back_onto_map() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "mole", 8, 10, 0);
    // Deep enough that the job is nowhere near done when the cancel lands.
    utils::wound(&mut app, depot, 90.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(app.world().get::<HiddenComponent>(worker).is_some());

    utils::stop_orders(app.world_mut(), worker);
    utils::run_ticks(&mut app, 1);

    assert!(
        app.world().get::<HiddenComponent>(worker).is_none(),
        "cancelling mid-job puts the worker back on the map"
    );
    utils::assert_adjacent_to_footprint(app.world_mut(), worker, depot);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn patient_destroyed_mid_repair_finishes_order_and_frees_worker() {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "mole", 8, 10, 0);
    utils::wound(&mut app, depot, 90.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert!(app.world().get::<HiddenComponent>(worker).is_some());

    // The patient goes down under the hands mending it.
    spawn::destroy_entity(app.world_mut(), depot);
    utils::run_ticks(&mut app, 2);

    assert!(
        app.world().get::<HiddenComponent>(worker).is_none(),
        "losing the patient puts the worker back on the map"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), worker),
        "with nothing left of its order"
    );
}

#[test]
fn mender_in_open_follows_patient_that_walks_away() {
    let mut app = app();
    let (patient, patient_id) = utils::spawn_owned(&mut app, "casualty", 10, 10, 0);
    let (orderly, orderly_id) = utils::spawn_owned(&mut app, "orderly", 9, 10, 0);
    utils::wound(&mut app, patient, 60.0);

    repair(&mut app, orderly_id, patient_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let mended = utils::current_health(&app, patient);
    assert!(
        mended > FixedU64::from_num(40),
        "the orderly reached its patient and started work"
    );

    // Walk the patient out of reach. A mender that works in the open has settled
    // nothing, so it keeps closing the distance rather than treating from afar. At
    // equal speed it only draws level once the patient stops, so allow for the walk
    // and the twelve ticks of work.
    utils::select(&mut app, patient_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 60);

    assert_eq!(
        utils::current_health(&app, patient),
        FixedU64::from_num(100),
        "the work carried on once the orderly caught up"
    );
    let gap = astar::chebyshev(
        utils::cell_of(app.world_mut(), orderly),
        utils::cell_of(app.world_mut(), patient),
    );
    assert!(
        gap <= 1,
        "and it followed rather than staying put, gap {gap}"
    );
}

#[test]
fn mender_stops_at_near_side_of_target() {
    let mut app = app();
    // The depot spans (10, 10) to (11, 11) and the worker comes at it from the east.
    // Closing on the position alone would send it round to the north-west corner,
    // since that is the cell the position names.
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 18, 11, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, 40);

    let stopped = utils::cell_of(app.world_mut(), worker);
    assert_eq!(
        stopped.x, 12,
        "it stopped against the east face it walked up to, at {stopped:?}"
    );
    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "and mended from there"
    );
}

#[test]
fn mender_faces_patient_rather_than_corner_its_position_names() {
    let mut app = app();
    // Level with the depot's lower row and just east of it, so the patient lies
    // due west. Its position names the north-west cell, which does not.
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 12, 11, 0);
    utils::wound(&mut app, depot, 20.0);
    utils::grant_gold(&mut app, 500);

    repair(&mut app, worker_id, depot_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let facing = app.world().get::<LocationComponent>(worker).unwrap().facing;
    assert!(
        facing.x < FixedI64::ZERO,
        "it faces west toward the depot, got {facing:?}"
    );
    assert_eq!(
        facing.y,
        FixedI64::ZERO,
        "and squarely so: aiming at the position would tilt it north, got {facing:?}"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player plus an enemy, a 100-point `depot` built in 20 ticks for 200
/// gold, a `hall` that mends in half its build time, a `monolith` nothing produces,
/// an untagged `soldier`, and four repairers differing only in their terms.
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");

        registry.register(building("depot", None));
        registry.register(building("hall", Some(FixedU64::from_num(0.5))));
        // Tagged and damageable, but nothing produces it, so nothing paces a repair.
        registry.register(
            EntityTypeDef::new("monolith")
                .with_location(utils::GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_tags(["building"]),
        );
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_train_time(20),
        );
        // A patient that can walk away mid-treatment, and the field medic that mends
        // it — the pair that shows a mender following its work.
        registry.register_tag("flesh");
        registry.register(
            EntityTypeDef::new("casualty")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(100)
                .with_train_time(20)
                .with_tags(["flesh"]),
        );
        registry.register(
            EntityTypeDef::new("orderly")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_repairer(
                    ["flesh"],
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present,
                    false,
                    RepairCost::Free,
                    None,
                ),
        );

        registry.register(repairer(
            "worker",
            WorkPresence::PresentStacking,
            Some(5),
            RepairCost::ProRata,
        ));
        registry.register(repairer(
            "loner",
            WorkPresence::Present,
            Some(5),
            RepairCost::ProRata,
        ));
        registry.register(repairer(
            "mole",
            WorkPresence::Hidden,
            Some(5),
            RepairCost::ProRata,
        ));
        // No patience limit: it waits at the job however long it takes.
        registry.register(repairer(
            "stoic",
            WorkPresence::PresentStacking,
            None,
            RepairCost::ProRata,
        ));
        // Pays out of its own energy at a flat rate, works alone, and reaches two
        // cells — the field-medic shape rather than the workshop one.
        registry.register(
            EntityTypeDef::new("medic")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_energy(50, FixedU64::ZERO)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::from_num(2))
                .with_repairer(
                    ["building"],
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present,
                    false,
                    RepairCost::Energy(FixedU64::from_num(0.5)),
                    None,
                ),
        );
        // Charges a flat rate for every tick it works, rather than a share of the
        // target's price.
        registry.register(repairer(
            "hauler",
            WorkPresence::PresentStacking,
            Some(5),
            RepairCost::PerTick(resources::cost([("gold", 1)])),
        ));
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// A 2×2, 100-point structure costing 200 gold and built in 20 ticks, optionally
/// declaring how its repair time relates to that.
fn building(name: &str, repair_ratio: Option<FixedU64>) -> EntityTypeDef {
    let def = EntityTypeDef::new(name)
        .with_location(utils::GROUND, NavSize::new(2, 2), Solidity::Solid)
        .with_health(100)
        .with_cost([("gold", 200)])
        .with_build_time(20)
        .with_tags(["building"]);
    match repair_ratio {
        Some(ratio) => def.with_repair_ratio(ratio),
        None => def,
    }
}

/// Gold spent mending a 40-point hole in one depot with the given crew, all of them
/// charging a flat rate per tick.
fn per_tick_spend(crew: &[&str]) -> u32 {
    let mut app = app();
    let (depot, depot_id) = utils::spawn_owned(&mut app, "depot", 10, 10, 0);
    utils::wound(&mut app, depot, 40.0);
    utils::grant_gold(&mut app, 500);

    for (index, type_name) in crew.iter().enumerate() {
        let (_, worker) = utils::spawn_owned(&mut app, type_name, 8, 10 + index as u32, 0);
        repair(&mut app, worker, depot_id);
    }
    utils::run_ticks(&mut app, utils::APPLY + 30);

    assert_eq!(
        utils::current_health(&app, depot),
        FixedU64::from_num(100),
        "the crew finishes the job"
    );
    500 - utils::gold(app.world())
}

/// A worker that mends buildings at the rate they are built, on the given terms.
fn repairer(
    name: &str,
    presence: WorkPresence,
    patience: Option<u32>,
    cost: RepairCost,
) -> EntityTypeDef {
    let def = EntityTypeDef::new(name)
        .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
        .with_movement(FixedU64::from_num(0.5))
        .with_health(20)
        .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
        .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE);
    // The factor only means something to a pro-rata bill, and declaring it without
    // one is rejected at registration.
    let def = match cost {
        RepairCost::ProRata => {
            def.with_stat(EntityStatId::REPAIR_COST_FACTOR, FixedU64::from_num(0.5))
        }
        _ => def,
    };
    def.with_repairer(
        ["building"],
        RepairRate::Production,
        presence,
        false,
        cost,
        patience,
    )
}

/// Selects `worker` and orders it to mend `target`.
fn repair(app: &mut App, worker: SimulationId, target: SimulationId) {
    utils::select(app, worker);
    utils::push_command(
        app,
        PlayerCommand::Repair {
            target,
            flush: true,
        },
    );
}

/// The crew a target records, if anybody is mending it.
fn crew_of(app: &App, target: Entity) -> Option<BTreeSet<SimulationId>> {
    app.world()
        .get::<UnderRepairComponent>(target)
        .map(|crew| crew.repairers.clone())
}

/// The worker's current energy.
fn energy(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<EnergyComponent>(entity)
        .unwrap()
        .current()
}

/// Refills `amount` energy directly, standing in for a pool that regenerated.
fn grant_energy(app: &mut App, entity: Entity, amount: f64) {
    let max = app
        .world()
        .get::<StatsComponent>(entity)
        .unwrap()
        .effective(EntityStatId::MAX_ENERGY)
        .unwrap();
    app.world_mut()
        .get_mut::<EnergyComponent>(entity)
        .unwrap()
        .regenerate(FixedU64::from_num(amount), max);
}

/// Spends `amount` energy directly, to set up a worker that is nearly spent.
fn drain_energy(app: &mut App, entity: Entity, amount: f64) {
    assert!(
        app.world_mut()
            .get_mut::<EnergyComponent>(entity)
            .unwrap()
            .spend(FixedU64::from_num(amount)),
        "the worker had that much energy to spend"
    );
}
