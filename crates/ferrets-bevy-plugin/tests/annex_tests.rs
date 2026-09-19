//! Annexes: buildings that stand in a neighbour's dock. The bond is derived
//! from what stands where, so raising, landing, lifting off and dying all
//! settle it; what an annex does without a primary, and who may take one, are
//! its own content.

mod utils;

use bevy::prelude::*;
use ferrets_content::registry::ContentRegistry;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_simulation::{
    annex,
    command::PlayerCommand,
    components::{
        annex::{AnnexComponent, Docking, DocksComponent},
        build::{SiteWork, UnderConstructionComponent},
        location::LocationComponent,
        research::ResearchComponent,
    },
    entity_def::{self, Operation, Switch},
    events::{DeathCause, SimulationEvent},
    player_research::PlayerResearch,
    session::player_id::PlayerId,
    simulation_id::SimulationId,
};

//
// ─── Raising an annex ─────────────────────────────────────────────────────────
//

#[test]
fn annex_rises_on_its_primarys_dock_and_docks_when_it_stands() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    // The dock sits two cells along the keep's own footprint.
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let lookout = utils::single_owned_of_type(app.world_mut(), "lookout", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(lookout)
            .is_some(),
        "the site is up but unfinished"
    );
    // An unfinished annex is nobody's annex yet.
    assert_eq!(primary_of(&app, lookout), Docking::Alone);
    assert!(
        app.world()
            .get::<DocksComponent>(keep)
            .expect("a primary that offers docks carries the component")
            .annexes
            .is_empty(),
        "its dock is empty until the annex stands"
    );

    // Four ticks of build time, and the tick after it stands the pass docks it.
    utils::run_ticks(&mut app, 5);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(lookout)
            .is_none(),
        "the annex is finished"
    );
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));
    let docked = app
        .world()
        .get::<DocksComponent>(keep)
        .expect("the primary holds what stands in its docks");
    assert_eq!(docked.annexes.len(), 1);
    assert_eq!(
        docked.annexes.get(&0).copied(),
        Some(entity_def::simulation_id(app.world(), lookout))
    );
}

#[test]
fn primary_holds_one_annex_in_each_of_its_docks() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    // The keep offers two: (2, 0) east of it and (0, 2) south of it.
    let east = docked_annex(&mut app, keep_id, "lookout", 12, 10);
    let south = docked_annex(&mut app, keep_id, "beacon", 10, 12);

    assert_eq!(primary_of(&app, east), Docking::Primary(keep_id));
    assert_eq!(primary_of(&app, south), Docking::Primary(keep_id));

    // Each is recorded against the dock it fills, so the keep knows both
    // rather than only the last one settled.
    let docks = app
        .world()
        .get::<DocksComponent>(keep)
        .expect("it holds annexes")
        .annexes
        .clone();
    assert_eq!(docks.len(), 2);
    assert_eq!(
        docks.get(&0).copied(),
        Some(entity_def::simulation_id(app.world(), east))
    );
    assert_eq!(
        docks.get(&1).copied(),
        Some(entity_def::simulation_id(app.world(), south))
    );
}

#[test]
fn annex_rises_nowhere_but_on_dock() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);

    // Beside the dock rather than on it: the keep is 2x2 at (10, 10), so this
    // cell is one away from its footprint and well within its build range —
    // the placement rule is the only thing that can refuse it.
    order_annex(&mut app, keep_id, "lookout", 12, 11);
    utils::run_ticks(&mut app, utils::APPLY + 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "lookout"), 0);
    // The price was never drawn, since no site was ever placed.
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn dying_annex_holds_its_dock_against_another() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 20);
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let lookout = utils::single_owned_of_type(app.world_mut(), "lookout", 0);

    // Shot down, it spends its dying phase on the map — and holds the cells of
    // the dock while it does, so nothing rises in its place.
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), lookout, DeathCause::Depleted);
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "lookout"),
        1,
        "the dying one is the only one"
    );

    // Once its remains are gone the dock is free, and the next one rises.
    utils::run_ticks(&mut app, 5);
    utils::assert_despawned(app.world_mut(), lookout);
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let raised = utils::single_owned_of_type(app.world_mut(), "lookout", 0);
    assert_ne!(raised, lookout);
    assert_eq!(primary_of(&app, raised), Docking::Primary(keep_id));
}

#[test]
fn training_waits_behind_raising_annex() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 20);
    order_annex(&mut app, keep_id, "watchpost", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = utils::single_owned_of_type(app.world_mut(), "watchpost", 0);

    // The keep raises the annex itself, so its one order queue holds the Build
    // in front and a unit queued behind it waits: both are paid for at once —
    // ten gold each of the twenty — and nothing comes out until the annex
    // stands.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: keep_id,
            type_name: "runner".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::gold(app.world()), 0);
    assert_eq!(utils::train_queue_len(app.world(), keep), 1);
    // Eight of the watchpost's twelve ticks are still owed, and no unit comes
    // out in any of them — the runner's two ticks of training would have been
    // over four times inside this window had the queue not held it behind the
    // Build.
    for _ in 0..8 {
        assert_eq!(
            utils::count_of_type(app.world_mut(), "runner"),
            0,
            "no unit comes out while the annex goes up"
        );
        utils::run_ticks(&mut app, 1);
    }
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(site)
            .is_none(),
        "the annex stands"
    );

    // Two ticks of training once the queue reaches it.
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::count_of_type(app.world_mut(), "runner"), 1);
}

#[test]
fn primary_between_cells_raises_its_annex_on_whole_cell() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    // Standing past the middle of its cell, as a form set down there by hand
    // would: the position floors to ten and rounds to eleven, so the dock can
    // only land where the rounding rule puts it.
    app.world_mut()
        .get_mut::<LocationComponent>(keep)
        .unwrap()
        .position = FixedUVec2::new(utils::fixed("10.6"), utils::fixed("10.0"));
    utils::grant_gold(&mut app, 10);

    // Its dock is read from the cell its footprint anchors to, not from the
    // fraction it stands on, so it still names a whole cell.
    let lookout = app
        .world()
        .resource::<ContentRegistry>()
        .entity("lookout")
        .expect("lookout is registered");
    let dock = annex::dock_anchor_for(app.world(), keep, lookout).expect("the keep offers one");
    assert_eq!(dock, CellPos::new(13, 10));

    order_annex(&mut app, keep_id, "lookout", dock.x, dock.y);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let lookout = utils::single_owned_of_type(app.world_mut(), "lookout", 0);
    assert_eq!(
        entity_def::position(app.world(), lookout),
        FixedUVec2::from(dock),
        "the annex stands on the cell, whatever its primary is standing on"
    );
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));
}

//
// ─── Lifting off and landing ──────────────────────────────────────────────────
//

#[test]
fn lifting_off_orphans_annex_and_switches_it_off() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));
    assert_eq!(
        entity_def::operation(app.world(), lookout),
        Operation::Operating
    );

    command_morph(&mut app, keep_id, "keep_aloft");
    // Three ticks for the command, four of morph time, and the pass that tick.
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        utils::single_owned_of_type(app.world_mut(), "keep_aloft", 0),
        keep,
        "the keep is the same entity aloft"
    );
    assert_eq!(primary_of(&app, lookout), Docking::Alone);
    assert!(
        app.world().get::<DocksComponent>(keep).is_none(),
        "aloft it offers no docks, so it holds no record of any"
    );
    // Idle without a primary: it stands, and it does not work.
    assert_eq!(
        entity_def::operation(app.world(), lookout),
        Operation::Disabled(Switch::Alone)
    );
}

#[test]
fn landing_beside_orphan_docks_it_again() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);

    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(primary_of(&app, lookout), Docking::Alone);

    // Down again on the same ground: the dock resolves to the lookout's cell
    // once more, so it is the keep's again.
    command_morph(&mut app, keep_id, "keep");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        utils::single_owned_of_type(app.world_mut(), "keep", 0),
        keep
    );
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));
    assert_eq!(
        entity_def::operation(app.world(), lookout),
        Operation::Operating
    );
}

#[test]
fn lift_off_leaves_researching_annex_to_hold_its_work() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    let lookout_id = entity_def::simulation_id(app.world(), lookout);
    let signals = app
        .world()
        .resource::<ContentRegistry>()
        .research("signals")
        .expect("the fixture registers signals");

    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::StartResearch {
            researcher: lookout_id,
            research: signals,
        },
    );
    // Three ticks for the command to land, and four of work on it — the tick
    // it landed included — of a twenty-tick topic.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    let progress = research_progress(&app, lookout);
    assert_eq!(progress, 4);

    // The keep leaves anyway: what its annex is doing is the annex's own
    // business. The work goes on while the change runs — the keep is still on
    // the ground for that — and stops the tick it is orphaned: ten of the
    // twenty ticks are done by then.
    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "keep_aloft"),
        1,
        "the lift-off was not refused"
    );
    assert_eq!(primary_of(&app, lookout), Docking::Alone);
    assert_eq!(
        entity_def::operation(app.world(), lookout),
        Operation::Disabled(Switch::Alone)
    );
    let held = research_progress(&app, lookout);
    assert_eq!(held, 10);

    // A lookout idles alone, so the work waits where it stood, however long
    // it is left.
    utils::run_ticks(&mut app, 20);
    assert_eq!(
        research_progress(&app, lookout),
        held,
        "held where it was, and no further"
    );
    assert!(
        !app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, signals)
    );

    // Landed again, it takes the work up from where it stopped: the ten ticks
    // still owed of the twenty.
    command_morph(&mut app, keep_id, "keep");
    utils::run_ticks(&mut app, utils::APPLY + 5 + 10);
    assert!(
        app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, signals),
        "the next primary finished what the first walked out on"
    );
}

#[test]
fn orphaned_annex_takes_no_new_research() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    let lookout_id = entity_def::simulation_id(app.world(), lookout);
    let signals = app
        .world()
        .resource::<ContentRegistry>()
        .research("signals")
        .expect("the fixture registers signals");

    // Orphaned before anything is asked of it.
    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(primary_of(&app, lookout), Docking::Alone);
    assert_eq!(
        entity_def::operation(app.world(), lookout),
        Operation::Disabled(Switch::Alone)
    );

    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::StartResearch {
            researcher: lookout_id,
            research: signals,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 20);

    // An idling annex is idle: the topic never starts, so nothing is held for
    // a later primary to finish, and the ten gold is still the player's.
    assert!(
        app.world().get::<ResearchComponent>(lookout).is_none(),
        "nothing was started"
    );
    assert!(
        !app.world()
            .resource::<PlayerResearch>()
            .is_completed(0, signals)
    );
    assert_eq!(utils::gold(app.world()), 10);
}

#[test]
fn primary_landing_back_works_annex_it_left_halted() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 30);
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = utils::single_owned_of_type(app.world_mut(), "lookout", 0);
    // Ten of the thirty gold, taken when the site was raised.
    assert_eq!(utils::gold(app.world()), 20);

    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    // Halted with work still owed while its primary is away. This is the
    // observation the rest of the test rests on: a site that kept advancing on
    // its own would have finished during the flight, and every assertion below
    // would then hold for the wrong reason.
    assert!(
        matches!(
            app.world()
                .get::<UnderConstructionComponent>(site)
                .map(|site| &site.work),
            Some(SiteWork::Halted)
        ),
        "it waited for its primary rather than raising itself"
    );

    command_morph(&mut app, keep_id, "keep");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(entity_def::of(app.world(), keep).name, "keep", "it landed");

    // Standing over its dock again is enough: the annex is raised by the
    // building it belongs to, so nobody has to order the work a second time.
    utils::run_ticks(&mut app, 12);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(site)
            .is_none(),
        "the annex it walked out on is finished"
    );
    assert_eq!(utils::count_of_type(app.world_mut(), "lookout"), 1);
    assert_eq!(primary_of(&app, site), Docking::Primary(keep_id));
    assert_eq!(utils::gold(app.world()), 20, "and paid for once");
}

#[test]
fn annex_site_is_finished_by_primary_that_did_not_found_it() {
    let mut app = utils::annex_app();
    let (_, founder_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, founder_id, "watchpost", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = utils::single_owned_of_type(app.world_mut(), "watchpost", 0);
    assert_eq!(utils::gold(app.world()), 0, "ten gold, taken at the raise");

    // The keep that founded it leaves for good, freeing the ground it stood on.
    command_morph(&mut app, founder_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert!(
        matches!(
            app.world()
                .get::<UnderConstructionComponent>(site)
                .map(|site| &site.work),
            Some(SiteWork::Halted)
        ),
        "the site stands halted with nobody offering its dock"
    );

    // Another keep of the same owner takes that ground, so the same cell is a
    // dock of its own: what the founder began, whatever stands over it
    // finishes. A half-raised annex is a claim on a spot, not on a building.
    let (_, heir_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::run_ticks(&mut app, 12);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(site)
            .is_none(),
        "the keep that never ordered it raised it"
    );
    assert_eq!(primary_of(&app, site), Docking::Primary(heir_id));
    assert_eq!(utils::gold(app.world()), 0, "and nobody paid a second time");
}

#[test]
fn lifting_off_mid_build_leaves_annex_site_halted() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = utils::single_owned_of_type(app.world_mut(), "lookout", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(site)
            .is_some(),
        "the site is up and unfinished"
    );

    // Raising an annex is a Build order, and a Build does not stand through a
    // soft cancel: the change of form flushes it and the keep leaves.
    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        utils::single_owned_of_type(app.world_mut(), "keep_aloft", 0),
        keep,
        "the keep lifted off mid-build"
    );
    assert!(
        matches!(
            app.world()
                .get::<UnderConstructionComponent>(site)
                .map(|site| &site.work),
            Some(SiteWork::Halted)
        ),
        "its half-raised annex stands halted, for any builder to take up"
    );
}

//
// ─── What an annex does with no primary ───────────────────────────────────────
//

#[test]
fn razed_annex_goes_when_its_primary_does() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "beacon", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let beacon = utils::single_owned_of_type(app.world_mut(), "beacon", 0);
    assert_eq!(primary_of(&app, beacon), Docking::Primary(keep_id));

    utils::record_announcements(&mut app);
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), keep, DeathCause::Depleted);
    utils::run_ticks(&mut app, 1);

    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied {
                    cause: DeathCause::Decayed,
                    ..
                }
            )),
        "it went as decayed: nothing sustained it"
    );
    // Its own dying phase, and it is off the map.
    utils::run_ticks(&mut app, 5);
    utils::assert_despawned(app.world_mut(), beacon);
}

#[test]
fn razed_annex_site_goes_when_its_dock_does() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "beacon", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let site = utils::single_owned_of_type(app.world_mut(), "beacon", 0);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(site)
            .is_some(),
        "one of the beacon's four ticks is in, so the site still stands unfinished"
    );

    // Its primary goes, so nothing offers the dock the site stands on. A
    // beacon cannot stand without a primary, and a half-raised one answers to
    // the same terms: it goes the way the finished building would.
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), keep, DeathCause::Depleted);
    // One tick for the dock to be gone and the site with it, then the two the
    // beacon takes to die.
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), site);
}

#[test]
fn fading_annex_loses_health_until_it_dies() {
    let mut app = utils::annex_app();
    let (keep, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "mast", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let mast = utils::single_owned_of_type(app.world_mut(), "mast", 0);
    // Ten health, less the one tick it fades on the way in: the stats it is
    // judged by are folded at the head of the tick, and the bond is derived
    // after the orders, so the tick a fading annex finishes and docks is a
    // tick it spent standing alone. Two health, once.
    assert_eq!(utils::health(&app, mast), 8);
    let docked = utils::health(&app, mast);
    utils::run_ticks(&mut app, 3);
    assert_eq!(
        utils::health(&app, mast),
        docked,
        "docked, it fades no further"
    );

    // A mast keeps working alone, so losing its primary costs it health only.
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), keep, DeathCause::Depleted);
    utils::run_ticks(&mut app, 2);
    assert_eq!(primary_of(&app, mast), Docking::Alone);
    assert_eq!(
        entity_def::operation(app.world(), mast),
        Operation::Operating,
        "it works without a primary, it only fades"
    );
    // Two health a tick from the eight it had, one tick behind the loss: the
    // tick its primary went was folded while it still had one, so 8 - 2 = 6
    // after two ticks, then 4, 2, and empty.
    assert_eq!(utils::health(&app, mast), 6);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, mast), 4);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, mast), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, mast), 0);
    utils::run_ticks(&mut app, 5);
    utils::assert_despawned(app.world_mut(), mast);
}

//
// ─── Who may take an annex ────────────────────────────────────────────────────
//

#[test]
fn allied_annex_refuses_rival() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let mooring = docked_annex(&mut app, keep_id, "mooring", 12, 10);
    assert_eq!(primary_of(&app, mooring), Docking::Primary(keep_id));

    // A rival takes the ground the keep held. An allied claim admits an ally's
    // primary and nobody else's, so the mooring is left standing alone rather
    // than changing hands.
    hand_over(&mut app, keep_id, 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(primary_of(&app, mooring), Docking::Alone);
    assert_eq!(
        entity_def::owner(app.world(), mooring),
        Some(0),
        "an allied annex never changes hands"
    );
}

#[test]
fn rival_landing_beside_seized_annex_takes_it() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    let lookout_id = entity_def::simulation_id(app.world(), lookout);
    utils::select(&mut app, lookout_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::selection(&app), vec![lookout_id]);

    utils::record_announcements(&mut app);
    let rival = hand_over(&mut app, keep_id, 1);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        entity_def::owner(app.world(), lookout),
        Some(1),
        "whoever docks with it owns it"
    );
    assert_eq!(
        primary_of(&app, lookout),
        Docking::Primary(entity_def::simulation_id(app.world(), rival))
    );
    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityCaptured {
                    from: Some(0),
                    to: 1,
                    ..
                }
            )),
        "the capture was announced"
    );
    assert!(
        utils::selection(&app).is_empty(),
        "it left the old owner's selection"
    );
}

#[test]
fn seizing_primary_that_is_itself_annex_takes_what_stands_in_its_dock() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    // A chain: the keep's dock holds a hub, and the hub's own dock holds a
    // relay. Both are seized, so both follow whoever takes the one above.
    let hub = docked_annex(&mut app, keep_id, "hub", 12, 10);
    let hub_id = entity_def::simulation_id(app.world(), hub);
    let relay = docked_annex(&mut app, hub_id, "relay", 13, 10);
    assert_eq!(primary_of(&app, hub), Docking::Primary(keep_id));
    assert_eq!(primary_of(&app, relay), Docking::Primary(hub_id));

    let rival = hand_over(&mut app, keep_id, 1);
    utils::run_ticks(&mut app, 1);

    // The hub changed hands because its own primary did. The relay's bond
    // never changed — it still stands in the same hub's dock — so only the
    // claim, judged afresh, can carry it across.
    assert_eq!(
        entity_def::owner(app.world(), hub),
        Some(1),
        "the hub followed the keep that took it"
    );
    assert_eq!(
        primary_of(&app, hub),
        Docking::Primary(entity_def::simulation_id(app.world(), rival))
    );
    assert_eq!(
        primary_of(&app, relay),
        Docking::Primary(hub_id),
        "the relay stands in the same dock throughout"
    );
    assert_eq!(
        entity_def::owner(app.world(), relay),
        Some(1),
        "and follows the hub it stands with"
    );
}

#[test]
fn bound_annex_refuses_rival() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "spire", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let spire = utils::single_owned_of_type(app.world_mut(), "spire", 0);
    assert_eq!(primary_of(&app, spire), Docking::Primary(keep_id));

    // A rival's keep on the same dock docks with nothing: a bound annex knows
    // only its own side's buildings, and stands alone rather than change hands.
    hand_over(&mut app, keep_id, 1);
    utils::run_ticks(&mut app, 1);
    assert_eq!(primary_of(&app, spire), Docking::Alone);
    assert_eq!(entity_def::owner(app.world(), spire), Some(0));

    // An ally's keep is no more welcome: bound means its owner's own.
    let rival = utils::single_owned_of_type(app.world_mut(), "keep", 1);
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), rival, DeathCause::Depleted);
    utils::run_ticks(&mut app, 5);
    utils::create_owned(&mut app, "keep", 10, 10, 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(primary_of(&app, spire), Docking::Alone);
}

#[test]
fn allys_keep_docks_allied_annex_without_taking_it() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 10);
    order_annex(&mut app, keep_id, "mooring", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    let mooring = utils::single_owned_of_type(app.world_mut(), "mooring", 0);

    // Player 2 shares player 0's team, so its keep is admitted — and the
    // mooring stays player 0's.
    let ally = hand_over(&mut app, keep_id, 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        primary_of(&app, mooring),
        Docking::Primary(entity_def::simulation_id(app.world(), ally)),
    );
    assert_eq!(entity_def::owner(app.world(), mooring), Some(0));
}

#[test]
fn two_primaries_offering_one_dock_leave_annex_with_one() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);

    // A tower whose dock is below it, standing so that dock is the lookout's
    // cell too. Exactly one of the two ends up holding it, and the annex is
    // never in both docks at once. Which one it is cannot be told apart from
    // incumbency here, the keep being both the earlier and the lower id;
    // `lower_id_takes_dock_from_primary_that_held_it` is what separates them.
    let (_, tower_id) = utils::create_owned(&mut app, "tower", 12, 8, 0);
    utils::run_ticks(&mut app, 1);
    assert!(keep_id < tower_id, "the keep took the lower id");
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));
}

#[test]
fn lower_id_takes_dock_from_primary_that_held_it() {
    let mut app = utils::annex_app();
    // A keep in the air first, so it holds the *lower* id while offering no
    // dock at all; the tower that takes the lookout is the higher id. Ids run
    // in creation order, so landing is the only way a lower id can arrive at
    // a dock someone else already holds.
    let (_, keep_id) = utils::create_owned(&mut app, "keep_aloft", 10, 10, 0);
    let (_, tower_id) = utils::create_owned(&mut app, "tower", 12, 8, 0);
    let lookout = utils::create_owned(&mut app, "lookout", 12, 10, 0).0;
    utils::run_ticks(&mut app, 1);
    assert!(keep_id < tower_id, "the aloft keep took the lower id");
    assert_eq!(primary_of(&app, lookout), Docking::Primary(tower_id));

    // The keep lands over the same cell. A bond that simply stayed put would
    // leave the tower holding it; the id rule hands it to the keep.
    command_morph(&mut app, keep_id, "keep");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(
        primary_of(&app, lookout),
        Docking::Primary(keep_id),
        "the lowest id holds it, incumbent or not"
    );
}

//
// ─── What a docked annex unlocks ──────────────────────────────────────────────
//

#[test]
fn orphaned_lookout_gates_nothing_until_it_docks_again() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    utils::grant_gold(&mut app, 30);

    // The keep leaves, so nothing stands with the lookout at all.
    command_morph(&mut app, keep_id, "keep_aloft");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(primary_of(&app, lookout), Docking::Alone);

    // Aloft, the keep that owns the dock trains nothing, so the refusal has to
    // be asked of a second keep standing clear of the orphan. What that keep is
    // asked for is an annex docked with *it*, and an annex docked with nobody
    // is no more its own than one docked elsewhere.
    let (_, far_id) = utils::create_owned(&mut app, "keep", 20, 20, 0);
    utils::run_ticks(&mut app, 1);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: far_id,
            type_name: "sentry".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "sentry"),
        0,
        "an annex standing with nobody unlocks nothing"
    );

    // Landing is docking, so the gate comes back with the bond and no order
    // re-establishes it. There is no state where a landed keep stands beside
    // an orphan of its own: the tick it lands, the lookout is its again.
    command_morph(&mut app, keep_id, "keep");
    utils::run_ticks(&mut app, utils::APPLY + 5);
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));

    // Docked again, the sentry it gates comes: six ticks of training.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: keep_id,
            type_name: "sentry".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "sentry"), 1);
}

#[test]
fn sentry_needs_lookout_docked_to_keep_that_trains_it() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    utils::grant_gold(&mut app, 30);

    // No annex, no sentry: the requirement is asked of the trainer itself.
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: keep_id,
            type_name: "sentry".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "sentry"), 0);

    order_annex(&mut app, keep_id, "lookout", 12, 10);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: keep_id,
            type_name: "sentry".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "sentry"), 1);
}

#[test]
fn sentry_is_refused_by_keep_lookout_does_not_stand_with() {
    let mut app = utils::annex_app();
    let (_, keep_id) = utils::create_owned(&mut app, "keep", 10, 10, 0);
    let lookout = docked_lookout(&mut app, keep_id, 12, 10);
    utils::grant_gold(&mut app, 30);

    // A second keep of its own, far from the lookout's dock: it can train, and
    // the lookout is not docked with it.
    let (_, far_id) = utils::create_owned(&mut app, "keep", 20, 20, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(primary_of(&app, lookout), Docking::Primary(keep_id));

    utils::push_command(
        &mut app,
        PlayerCommand::TrainEntity {
            trainer: far_id,
            type_name: "sentry".into(),
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(
        utils::count_of_type(app.world_mut(), "sentry"),
        0,
        "the annex serves the keep it stands with, not the player"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Orders `primary` to raise `type_name` at `(x, y)`.
fn order_annex(app: &mut App, primary: SimulationId, type_name: &str, x: u32, y: u32) {
    utils::push_command(
        app,
        PlayerCommand::BuildEntity {
            builder: primary,
            type_name: type_name.into(),
            position: utils::pos(x, y),
            flush: false,
        },
    );
}

/// Ticks of work the entity has put into its research.
fn research_progress(app: &App, researcher: Entity) -> u32 {
    app.world()
        .get::<ResearchComponent>(researcher)
        .expect("a researching entity carries the component")
        .progress
}

/// Selects the entity and commands its change of form, so the executor judges
/// it the way a player's click would.
fn command_morph(app: &mut App, entity: SimulationId, type_name: &str) {
    utils::select(app, entity);
    utils::push_command(
        app,
        PlayerCommand::Morph {
            type_name: type_name.to_string(),
            flush: true,
        },
    );
}

/// The primary an annex stands with, by simulation id.
fn primary_of(app: &App, annex: Entity) -> Docking {
    app.world()
        .get::<AnnexComponent>(annex)
        .expect("an annex carries the component fitted from its type")
        .docked_to
}

fn docked_annex(app: &mut App, primary: SimulationId, type_name: &str, x: u32, y: u32) -> Entity {
    utils::grant_gold(app, 10);
    order_annex(app, primary, type_name, x, y);
    utils::run_ticks(app, utils::APPLY + 6);
    let owner = entity_def::owner(
        app.world(),
        app.world()
            .resource::<ferrets_simulation::entity_index::EntityIndex>()
            .alive(primary)
            .expect("the primary stands"),
    )
    .expect("the primary has an owner");
    utils::single_owned_of_type(app.world_mut(), type_name, owner)
}

/// Raises a lookout on the keep's dock and runs until it stands.
fn docked_lookout(app: &mut App, keep_id: SimulationId, x: u32, y: u32) -> Entity {
    utils::grant_gold(app, 10);
    order_annex(app, keep_id, "lookout", x, y);
    utils::run_ticks(app, utils::APPLY + 6);
    utils::single_owned_of_type(app.world_mut(), "lookout", 0)
}

/// Lifts player 0's keep away from its annex and lands `taker`'s keep on the
/// dock the annex stands in.
fn hand_over(app: &mut App, keep_id: SimulationId, taker: PlayerId) -> Entity {
    command_morph(app, keep_id, "keep_aloft");
    utils::run_ticks(app, utils::APPLY + 5);
    // Flying off out of the way, so the ground its dock covered is free.
    let aloft = utils::single_owned_of_type(app.world_mut(), "keep_aloft", 0);
    ferrets_simulation::spawn::despawn_entity(app.world_mut(), aloft, DeathCause::Depleted);
    utils::run_ticks(app, 1);
    utils::create_owned(app, "keep", 10, 10, taker).0
}
