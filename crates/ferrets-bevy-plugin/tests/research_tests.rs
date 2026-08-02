//! Research and requirements: a research pays at issue, is worked by its
//! hosting building, completes once per player, and lands its buff through the
//! ordinary stat fold; requirement lists gate production and research commands
//! against what currently stands and what has been researched.

mod utils;

use bevy::prelude::App;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand, components::build::UnderConstructionComponent,
    player_research::PlayerResearch, simulation_id::SimulationId, spawn,
};

//
// ─── Research lifecycle ─────────────────────────────────────────────────────
//

#[test]
fn research_pays_at_issue_and_completes() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // Paid up front, not yet done.
    assert_eq!(utils::gold(app.world()), 20);
    assert!(!completed(&app, "smithing"));

    utils::run_ticks(&mut app, 15);
    assert!(completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 20);
}

#[test]
fn completed_research_buffs_existing_and_new_units() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (veteran, _) = utils::spawn_owned(&mut app, "pikeman", 5, 5, 0);
    utils::grant_gold(&mut app, 50);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_damage(&app, veteran),
        FixedU64::from_num(10)
    );

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));

    // The buff reaches the unit that predates the research...
    assert_eq!(
        utils::effective_damage(&app, veteran),
        FixedU64::from_num(15)
    );

    // ...and one spawned after it, through the same per-tick fold.
    let (recruit, _) = utils::spawn_owned(&mut app, "pikeman", 6, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_damage(&app, recruit),
        FixedU64::from_num(15)
    );
}

#[test]
fn completed_research_refuses_repeat() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 70);

    // A repeat is refused before payment.
    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert_eq!(utils::gold(app.world()), 70);
}

#[test]
fn research_under_way_blocks_second_start() {
    let mut app = utils::research_app();
    let (_, first_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (_, second_id) = utils::spawn_owned(&mut app, "lab", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    // The second command lands while the first order is in flight, anywhere
    // in the player's holdings — one topic, one payment.
    start_research(&mut app, first_id, "smithing");
    start_research(&mut app, second_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 70);

    utils::run_ticks(&mut app, 15);
    assert!(completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 70);
}

#[test]
fn force_cancel_refunds_and_frees_topic() {
    let mut app = utils::research_app();
    let (lab, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::gold(app.world()), 20);

    utils::stop_orders(app.world_mut(), lab);
    utils::run_ticks(&mut app, 1);

    // The full price comes back and the progress is discarded.
    assert_eq!(utils::gold(app.world()), 50);
    assert!(!completed(&app, "smithing"));

    // The topic is free again: a fresh start runs to completion.
    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 20);
}

#[test]
fn researcher_death_refunds_and_frees_topic() {
    let mut app = utils::research_app();
    let (first, first_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (_, second_id) = utils::spawn_owned(&mut app, "lab", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    start_research(&mut app, first_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::gold(app.world()), 70);

    spawn::destroy_entity(app.world_mut(), first);
    utils::run_ticks(&mut app, 5);

    // Death force-cancels the queue, so the price comes back and the progress
    // is discarded — the same path a dying trainer's queue refunds through.
    assert!(!completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 100);

    // Nothing holds the topic: the second lab starts it fresh.
    start_research(&mut app, second_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 70);
}

#[test]
fn completion_outside_order_path_refunds_running_order() {
    let mut app = utils::research_app();
    let (lab, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::gold(app.world()), 20);

    // A completion landing outside the order path — a scenario script's grant.
    let smithing = utils::research_id(&app, "smithing");
    app.world_mut()
        .resource_mut::<PlayerResearch>()
        .complete(0, smithing);
    utils::run_ticks(&mut app, 2);

    // The running order has nothing left to work toward: it finishes and the
    // payment comes back rather than being silently consumed.
    assert_eq!(utils::gold(app.world()), 50);
    assert!(utils::order_queue_is_empty(app.world_mut(), lab));
}

//
// ─── Requirements ───────────────────────────────────────────────────────────
//

#[test]
fn research_requirement_gates_research() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    // Tactics requires smithing: refused outright, nothing paid.
    start_research(&mut app, lab_id, "tactics");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(!completed(&app, "tactics"));
    assert_eq!(utils::gold(app.world()), 100);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));

    start_research(&mut app, lab_id, "tactics");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "tactics"));
    assert_eq!(utils::gold(app.world()), 50);
}

#[test]
fn research_requirement_gates_training() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (guardhouse, guardhouse_id) = utils::spawn_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    // The halberdier waits on smithing: refused, nothing queued, nothing paid.
    train(&mut app, guardhouse_id, "halberdier");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "halberdier"), 0);
    assert_eq!(utils::train_queue_len(app.world(), guardhouse), 0);
    assert_eq!(utils::gold(app.world()), 100);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));

    train(&mut app, guardhouse_id, "halberdier");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "halberdier"), 1);
}

#[test]
fn tag_requirement_follows_standing_provider() {
    let mut app = utils::research_app();
    let (_, guardhouse_id) = utils::spawn_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    // The knight needs a "workshop" tag on something standing: nothing does.
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 0);
    assert_eq!(utils::gold(app.world()), 100);

    // A tagged building satisfies it...
    let (lab, _) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 1);
    assert_eq!(utils::gold(app.world()), 90);

    // ...and its loss re-derives to refused, with no bookkeeping in between.
    spawn::destroy_entity(app.world_mut(), lab);
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 1);
    assert_eq!(utils::gold(app.world()), 90);
}

#[test]
fn under_construction_provider_satisfies_nothing() {
    let mut app = utils::research_app();
    let (lab, _) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (_, guardhouse_id) = utils::spawn_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);
    app.world_mut()
        .entity_mut(lab)
        .insert(UnderConstructionComponent {
            progress: 0,
            builders: Default::default(),
        });

    // A workshop still going up unlocks nothing.
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 0);
    assert_eq!(utils::gold(app.world()), 100);

    app.world_mut()
        .entity_mut(lab)
        .remove::<UnderConstructionComponent>();
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 1);
}

#[test]
fn requirement_loss_keeps_queued_production() {
    let mut app = utils::research_app();
    let (lab, _) = utils::spawn_owned(&mut app, "lab", 10, 10, 0);
    let (guardhouse, guardhouse_id) = utils::spawn_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    // Queued while the workshop stood; the requirement gates only the command.
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::train_queue_len(app.world(), guardhouse), 1);

    spawn::destroy_entity(app.world_mut(), lab);
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 1);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Orders `researcher` to start the named research.
fn start_research(app: &mut App, researcher: SimulationId, name: &str) {
    let research = utils::research_id(app, name);
    utils::push_command(
        app,
        PlayerCommand::StartResearch {
            researcher,
            research,
        },
    );
}

/// Orders `trainer` to train one unit of `type_name`.
fn train(app: &mut App, trainer: SimulationId, type_name: &str) {
    utils::push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: type_name.into(),
        },
    );
}

/// Whether player 0 has completed the named research.
fn completed(app: &App, name: &str) -> bool {
    let research = utils::research_id(app, name);
    app.world()
        .resource::<PlayerResearch>()
        .is_completed(0, research)
}
