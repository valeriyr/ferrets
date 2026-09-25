//! Research and requirements: a research pays at issue, is worked by its
//! hosting building, completes once per player, and lands its buff through the
//! ordinary stat fold; requirement lists gate production and research commands
//! against what currently stands and what has been researched; and a player's
//! cancel gives the price back where losing the researcher does not.

mod utils;

use bevy::prelude::App;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::build::{SiteWork, UnderConstructionComponent},
    player_research::PlayerResearch,
    resources::PlayerResources,
    simulation_id::SimulationId,
    spawn,
};

//
// ─── Research lifecycle ─────────────────────────────────────────────────────
//

#[test]
fn research_pays_at_issue_and_completes() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
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
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (veteran, _) = utils::create_owned(&mut app, "pikeman", 5, 5, 0);
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
    let (recruit, _) = utils::create_owned(&mut app, "pikeman", 6, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_damage(&app, recruit),
        FixedU64::from_num(15)
    );
}

#[test]
fn completed_research_refuses_repeat() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
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
    let (_, first_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "lab", 20, 20, 0);
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
fn force_cancel_keeps_price_spent_and_frees_topic() {
    let mut app = utils::research_app();
    let (lab, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    // 50 granted − 30 for smithing.
    assert_eq!(utils::gold(app.world()), 20);

    utils::force_cancel_orders(app.world_mut(), lab);
    utils::run_ticks(&mut app, 1);

    // Work taken away rather than called off pays nothing back, so the 30
    // stays spent and the progress is discarded.
    assert_eq!(utils::gold(app.world()), 20);
    assert!(!completed(&app, "smithing"));

    // The topic is free again: a fresh start runs to completion.
    utils::grant_gold(&mut app, 30);
    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    // 20 left + 30 granted − 30 for the second attempt.
    assert_eq!(utils::gold(app.world()), 20);
}

#[test]
fn researcher_death_keeps_price_spent_and_frees_topic() {
    let mut app = utils::research_app();
    let (first, first_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "lab", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    start_research(&mut app, first_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    // 100 granted − 30 for smithing.
    assert_eq!(utils::gold(app.world()), 70);

    spawn::destroy_entity(app.world_mut(), first);
    utils::run_ticks(&mut app, 5);

    // A researcher lost takes its payment with it: the 30 stays spent and the
    // progress is discarded, as a dying trainer's queue does.
    assert!(!completed(&app, "smithing"));
    assert_eq!(utils::gold(app.world()), 70);

    // Nothing holds the topic: the second lab starts it fresh.
    start_research(&mut app, second_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    // 70 left − 30 for the second attempt.
    assert_eq!(utils::gold(app.world()), 40);
}

#[test]
fn completion_outside_order_path_refunds_running_order() {
    let mut app = utils::research_app();
    let (lab, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::gold(app.world()), 20);

    // A completion landing outside the order path — a scenario script's grant.
    let smithing = utils::research_id(&app, "smithing");
    app.world_mut()
        .resource_mut::<PlayerResearch>()
        .mark_completed(0, smithing);
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
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
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
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (guardhouse, guardhouse_id) = utils::create_owned(&mut app, "guardhouse", 20, 20, 0);
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
    let (_, guardhouse_id) = utils::create_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);

    // The knight needs a "workshop" tag on something standing: nothing does.
    train(&mut app, guardhouse_id, "knight");
    utils::run_ticks(&mut app, utils::APPLY + 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "knight"), 0);
    assert_eq!(utils::gold(app.world()), 100);

    // A tagged building satisfies it...
    let (lab, _) = utils::create_owned(&mut app, "lab", 10, 10, 0);
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
    let (lab, _) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (_, guardhouse_id) = utils::create_owned(&mut app, "guardhouse", 20, 20, 0);
    utils::grant_gold(&mut app, 100);
    app.world_mut()
        .entity_mut(lab)
        .insert(UnderConstructionComponent {
            progress: 0,
            work: SiteWork::Crew {
                builders: Default::default(),
            },
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
    let (lab, _) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    let (guardhouse, guardhouse_id) = utils::create_owned(&mut app, "guardhouse", 20, 20, 0);
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
// ─── Canceling ──────────────────────────────────────────────────────────────
//

#[test]
fn cancel_research_refunds_topic_and_frees_it() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    // 50 granted − 30 for smithing.
    assert_eq!(utils::gold(app.world()), 20);

    cancel_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // The player called it off, so the full 30 comes back.
    assert_eq!(utils::gold(app.world()), 50);
    assert!(!completed(&app, "smithing"));

    // The topic is free again: a fresh start runs to completion.
    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 15);
    assert!(completed(&app, "smithing"));
    // 50 − 30 for the second attempt.
    assert_eq!(utils::gold(app.world()), 20);
}

#[test]
fn cancel_research_ignores_topic_not_in_flight() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    // 50 granted − 30 for smithing.
    assert_eq!(utils::gold(app.world()), 20);

    // A topic this lab is not working pays nothing and stops nothing.
    cancel_research(&mut app, lab_id, "tactics");
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 20);

    utils::run_ticks(&mut app, 15);
    assert!(completed(&app, "smithing"));
}

#[test]
fn cancel_research_reaches_topic_still_waiting_its_turn() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 100);

    // Two topics on one lab: smithing runs, masonry waits behind it.
    start_research(&mut app, lab_id, "smithing");
    start_research(&mut app, lab_id, "masonry");
    utils::run_ticks(&mut app, utils::APPLY + 2);
    // 100 granted − 30 for smithing − 20 for masonry.
    assert_eq!(utils::gold(app.world()), 50);

    cancel_research(&mut app, lab_id, "masonry");
    utils::run_ticks(&mut app, utils::APPLY);
    // 50 + the 20 masonry cost, given back though it never started.
    assert_eq!(utils::gold(app.world()), 70);

    // Smithing is untouched and still completes; masonry never does.
    utils::run_ticks(&mut app, 25);
    assert!(completed(&app, "smithing"));
    assert!(!completed(&app, "masonry"));
}

#[test]
fn cancel_research_twice_in_one_frame_pays_once() {
    let mut app = utils::research_app();
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
    utils::grant_gold(&mut app, 50);

    start_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 4);
    // 50 granted − 30 for smithing.
    assert_eq!(utils::gold(app.world()), 20);

    // Both commands land in one frame: the second finds the entry the first
    // already marked and pays nothing for it.
    cancel_research(&mut app, lab_id, "smithing");
    cancel_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    // 20 + the 30 smithing cost, once: not 80.
    assert_eq!(utils::gold(app.world()), 50);
    utils::run_ticks(&mut app, 15);
    assert!(!completed(&app, "smithing"));
}

#[test]
fn cancel_research_beside_stop_in_one_frame_still_refunds_and_drops_topic() {
    for order in ["cancel then stop", "stop then cancel"] {
        let mut app = utils::research_app();
        let (_, lab_id) = utils::create_owned(&mut app, "lab", 10, 10, 0);
        utils::grant_gold(&mut app, 50);
        // The stop reads the selection, so the lab is selected ahead of it.
        utils::select(&mut app, lab_id);
        start_research(&mut app, lab_id, "smithing");
        utils::run_ticks(&mut app, utils::APPLY + 4);
        // 50 granted − 30 for smithing.
        assert_eq!(utils::gold(app.world()), 20, "{order}");

        // The stop's advisory mark neither hides the entry from the cancel
        // nor demotes the mandatory mark the cancel leaves, whichever lands
        // first: the topic is paid back once and dropped.
        let cancel = PlayerCommand::CancelResearch {
            researcher: lab_id,
            research: utils::research_id(&app, "smithing"),
        };
        let commands = match order {
            "cancel then stop" => [cancel, PlayerCommand::Stop],
            "stop then cancel" => [PlayerCommand::Stop, cancel],
            _ => unreachable!("the two orders above"),
        };
        for command in commands {
            utils::push_command(&mut app, command);
        }
        utils::run_ticks(&mut app, utils::APPLY + 1);

        // 20 + the 30 smithing cost.
        assert_eq!(utils::gold(app.world()), 50, "{order}");
        utils::run_ticks(&mut app, 15);
        assert!(!completed(&app, "smithing"), "{order}");
    }
}

#[test]
fn cancel_research_ignores_rival_researcher() {
    let mut app = utils::research_app_seating(utils::human_slots(2));
    // The rival's lab, started on smithing in the rival's own frame.
    let (_, lab_id) = utils::create_owned(&mut app, "lab", 20, 20, 1);
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(1, "gold", 50);
    let smithing = utils::research_id(&app, "smithing");
    let due = utils::tick(&app) + utils::APPLY;
    utils::run_ticks_commanding(
        &mut app,
        utils::APPLY + 1,
        1,
        due,
        vec![PlayerCommand::StartResearch {
            researcher: lab_id,
            research: smithing,
        }],
    );
    // 50 granted − 30 for smithing.
    assert_eq!(rival_gold(&app), 20);

    cancel_research(&mut app, lab_id, "smithing");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::gold(app.world()),
        0,
        "a player cancels only its own research, and is paid nothing for a rival's"
    );
    assert_eq!(rival_gold(&app), 20, "the rival keeps what it paid");
    utils::run_ticks(&mut app, 15);
    assert!(
        app.world()
            .resource::<PlayerResearch>()
            .is_completed(1, smithing),
        "and its topic completes"
    );
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

/// Calls off `name` on `researcher`.
fn cancel_research(app: &mut App, researcher: SimulationId, name: &str) {
    let research = utils::research_id(app, name);
    utils::push_command(
        app,
        PlayerCommand::CancelResearch {
            researcher,
            research,
        },
    );
}

/// Player 1's stockpile of gold.
fn rival_gold(app: &App) -> u32 {
    app.world().resource::<PlayerResources>().amount(1, "gold")
}
