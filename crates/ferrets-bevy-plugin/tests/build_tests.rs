//! Build order: workers raising construction sites.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::Solidity,
    registry::ContentRegistry, work::WorkPresence,
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        build::{BuildComponent, UnderConstructionComponent},
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
    },
    map::Map,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn build_constructs_building() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );

    // The worker walks to the site, pays, hides inside, and the building appears
    // under construction.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    {
        let world = app.world_mut();
        assert_eq!(utils::gold(world), 30);
        assert!(world.get::<HiddenComponent>(worker).is_some());
        assert_eq!(under_construction(world), 1);
    }

    // Construction completes: the marker is gone and the worker reappears next to
    // the building.
    utils::run_ticks(&mut app, 6);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    let world = app.world_mut();
    assert_eq!(under_construction(world), 0);

    let worker_cell = utils::cell_of(world, worker);
    let site = CellRect::new(CellPos::new(10, 10), CellSize::new(2, 2));
    assert!(
        world
            .resource::<Map>()
            .projection()
            .in_range_of_rect(worker_cell, site, 1)
    );
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));

    // A constructible type outside the worker's catalogue is rejected.
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "barracks".into(),
            position: utils::pos(20, 20),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::count_of_type(app.world_mut(), "barracks"), 0);
    assert_eq!(utils::gold(app.world_mut()), 30);
}

#[test]
fn cancelling_build_refunds_and_restores_builder() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );

    // Wait until construction has started (cost paid, builder hidden inside).
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::gold(app.world_mut()), 30);

    utils::stop_orders(app.world_mut(), worker);

    // The cancel destroys the unfinished building, refunds the cost, and the
    // builder reappears next to the site.
    utils::run_ticks(&mut app, 1);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
    assert_eq!(utils::gold(app.world_mut()), 80);
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn unaffordable_placement_finishes_order_without_site() {
    let mut app = utils::orders_app();
    // Already in reach of the site, so there is nothing to walk before the cost
    // check — and no gold is ever granted.
    let (mason, mason_id) = utils::spawn_owned(&mut app, "mason", 9, 10, 0);
    let stood_at = utils::cell_of(app.world_mut(), mason);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: mason_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(
        utils::count_of_type(app.world_mut(), "depot"),
        0,
        "nothing goes up without the gold to pay for it"
    );
    assert_eq!(utils::gold(app.world_mut()), 0, "and nothing was charged");
    assert_eq!(
        utils::cell_of(app.world_mut(), mason),
        stood_at,
        "the mason is left exactly where it stood"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), mason),
        "the order finished rather than waiting for funds"
    );
}

#[test]
fn site_destroyed_mid_construction_frees_builder() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // The site is destroyed under the builder working inside it.
    let site = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    spawn::destroy_entity(app.world_mut(), site);
    utils::run_ticks(&mut app, 2);

    assert!(
        app.world_mut().get::<HiddenComponent>(worker).is_none(),
        "losing the site puts the builder back on the map"
    );
    assert!(
        utils::order_queue_is_empty(app.world_mut(), worker),
        "with nothing left of its order"
    );
}

//
// ─── Where the builder stands ───────────────────────────────────────────────
//

#[test]
fn builder_working_in_open_stays_on_map() {
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::spawn_owned(&mut app, "mason", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: mason_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        utils::count_of_type(app.world_mut(), "depot"),
        1,
        "the site went up all the same"
    );
    assert!(
        app.world_mut().get::<HiddenComponent>(mason).is_none(),
        "a builder declared present raises the site from beside it instead of \
         vanishing into it"
    );

    // Still exposed for the whole job, then finishes normally.
    utils::run_ticks(&mut app, 6);
    assert!(app.world_mut().get::<HiddenComponent>(mason).is_none());
    assert_eq!(
        under_construction(app.world_mut()),
        0,
        "construction completed"
    );
}

#[test]
fn builder_walks_up_to_site_rather_than_into_it() {
    // The depot would span (10, 10) to (11, 11) and the mason approaches from the
    // east. Closing on the position alone would stop it a cell short of (10, 10) —
    // which is inside the footprint, where it would block its own site.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::spawn_owned(&mut app, "mason", 14, 11, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: mason_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(
        utils::count_of_type(app.world_mut(), "depot"),
        1,
        "the site went up, so nothing was standing in it"
    );
    let stopped = utils::cell_of(app.world_mut(), mason);
    assert_eq!(
        stopped.x, 12,
        "the mason stopped against the east face, at {stopped:?}"
    );
}

#[test]
fn long_reach_builder_raises_site_without_closing_in() {
    let mut app = surveyor_app();
    // The depot's footprint ends at (11, 11); the surveyor stands three cells east
    // of it, which its build_range of 3 already covers.
    let (surveyor, surveyor_id) = utils::spawn_owned(&mut app, "surveyor", 14, 10, 0);
    utils::grant_gold(&mut app, 80);
    let stood_at = utils::cell_of(app.world_mut(), surveyor);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: surveyor_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(
        utils::count_of_type(app.world_mut(), "depot"),
        1,
        "the site went up from three cells out"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), surveyor),
        stood_at,
        "a longer reach means no step toward the footprint at all"
    );
}

#[test]
fn builder_standing_on_site_blocks_it_and_is_never_moved() {
    // A builder that works in the open is left alone, so its own cells block the
    // footprint exactly as anything else standing there would. The order gives up
    // rather than shoving it out of the way.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::spawn_owned(&mut app, "mason", 10, 10, 0);
    utils::grant_gold(&mut app, 80);
    let stood_at = utils::cell_of(app.world_mut(), mason);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: mason_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        utils::count_of_type(app.world_mut(), "depot"),
        0,
        "the site is blocked by the builder itself"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        80,
        "and nothing was paid for it"
    );
    assert_eq!(
        utils::cell_of(app.world_mut(), mason),
        stood_at,
        "the builder is left exactly where it stood"
    );
    assert!(app.world_mut().get::<HiddenComponent>(mason).is_none());
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn builder_that_works_hidden_raises_site_it_stands_on() {
    // Leaving the map frees the cells, so a builder that disappears into its work can
    // raise a site from the spot it is standing on — the one thing the open worker
    // above cannot do.
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    // No walk is needed: the order lands, the worker steps inside, and the site goes
    // up on the cell it was standing on.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Six ticks of work later it is back out beside what it raised.
    utils::run_ticks(&mut app, 6);
    assert_eq!(under_construction(app.world_mut()), 0);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
}

#[test]
fn boxed_in_builder_finishes_site_and_waits_to_reappear() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::spawn_owned(&mut app, "worker", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Take away every cell it could come back out onto, then let the work finish.
    utils::set_all_cells_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 6);

    // The walls are up regardless: the building is not held back by having nowhere
    // to put the builder. The builder waits off the map with a queued reveal, and
    // comes back onto the one cell that frees.
    assert_eq!(
        under_construction(app.world_mut()),
        0,
        "the site finished on time"
    );
    utils::assert_reveal_deferred_then_lands_on(&mut app, worker, CellPos::new(9, 9));
}

#[test]
fn builder_faces_site_rather_than_corner_its_position_names() {
    let mut app = utils::orders_app();
    // Level with the depot's lower row and just east of it, so the building lies
    // due west. Its position names the north-west cell, which does not.
    let (mason, mason_id) = utils::spawn_owned(&mut app, "mason", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: mason_id,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);

    let facing = app.world().get::<LocationComponent>(mason).unwrap().facing;
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
// ─── Sharing a site ─────────────────────────────────────────────────────────
//

#[test]
fn crew_shares_one_site_and_each_member_adds_own_work() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::spawn_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);

    // Both start within reach, so the site goes up as soon as the orders land: one
    // places it and the other joins what it finds rather than failing on its cells.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        utils::gold(app.world_mut()),
        30,
        "only the builder that placed the site paid for it"
    );
    assert!(app.world_mut().get::<HiddenComponent>(first).is_none());
    assert!(app.world_mut().get::<HiddenComponent>(second).is_none());

    // Two builders on the site, two ticks of work a tick.
    let before = site_progress(app.world_mut());
    utils::run_ticks(&mut app, 1);
    assert_eq!(site_progress(app.world_mut()), before + 2);

    // A build time of six is served by a pair in three ticks of work.
    utils::run_ticks(&mut app, 2);
    assert_eq!(
        under_construction(app.world_mut()),
        0,
        "the crew finished the site between them"
    );
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
}

#[test]
fn sending_builder_to_unfinished_site_puts_it_to_work() {
    // The way a player actually asks for help: right-click the half-built thing,
    // rather than re-issuing the build command on its exact cell.
    let mut app = utils::orders_app();
    let (_, first_id) = utils::spawn_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(under_construction(app.world_mut()), 1);

    let site = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    let site_id = app
        .world_mut()
        .get::<EntityInfoComponent>(site)
        .unwrap()
        .id();
    utils::select(&mut app, second_id);
    utils::push_command(
        &mut app,
        PlayerCommand::SendToEntity {
            target: site_id,
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(
        app.world()
            .get::<BuildComponent>(second)
            .and_then(|build| build.building),
        Some(site_id),
        "the second builder took up the site rather than trailing after it"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        30,
        "and joining costs nothing on top of what the site was paid for"
    );
}

#[test]
fn builder_that_works_alone_turns_away_from_site_another_holds() {
    let mut app = utils::orders_app();
    let (_, first_id) = utils::spawn_owned(&mut app, "mason", 9, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "mason", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(
        utils::order_queue_is_empty(app.world_mut(), second),
        "a builder that works alone gives up on a site somebody else has"
    );

    // One builder, one tick of work a tick.
    let before = site_progress(app.world_mut());
    utils::run_ticks(&mut app, 1);
    assert_eq!(site_progress(app.world_mut()), before + 1);
}

#[test]
fn cancelling_one_of_crew_leaves_site_standing() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::spawn_owned(&mut app, "carpenter", 9, 10, 0);
    let (_, second_id) = utils::spawn_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(under_construction(app.world_mut()), 1);

    utils::stop_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        under_construction(app.world_mut()),
        1,
        "the builder left behind carries on with what they had raised together"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        30,
        "and nothing is refunded while the site still stands"
    );
}

#[test]
fn raised_site_records_crew_until_last_builder_leaves() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::spawn_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::spawn_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        crew_of_site(app.world_mut()),
        vec![first_id, second_id],
        "both carpenters show up in the site's crew"
    );

    // One of the pair stops. The site keeps standing, and keeps the other builder.
    utils::stop_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        crew_of_site(app.world_mut()),
        vec![second_id],
        "one builder leaving does not empty the site"
    );

    // The last builder out empties the crew and tears the site down, which then sees
    // out its dying phase like anything else destroyed.
    utils::stop_orders(app.world_mut(), second);
    utils::run_ticks(&mut app, 1);
    assert!(crew_of_site(app.world_mut()).is_empty());
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Orders `builder` to raise a depot on the one site the sharing suite uses.
fn order_depot(app: &mut App, builder: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::BuildEntity {
            builder,
            type_name: "depot".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
}

/// The economy content roster plus a `surveyor` that raises depots from three
/// cells out.
fn surveyor_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("surveyor")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(3))
                .with_builder(["depot"], WorkPresence::Present),
        );
    }
    utils::register_orders_content(&mut app);
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Ticks of work put into the sites on the map.
fn site_progress(world: &mut World) -> u32 {
    world
        .query::<&UnderConstructionComponent>()
        .iter(world)
        .map(|site| site.progress)
        .sum()
}

/// The crew on the one site the sharing suite raises, in [`SimulationId`] order.
fn crew_of_site(world: &mut World) -> Vec<SimulationId> {
    world
        .query::<&UnderConstructionComponent>()
        .iter(world)
        .flat_map(|site| site.builders.iter().copied())
        .collect()
}

/// How many sites are still going up.
fn under_construction(world: &mut World) -> usize {
    world
        .query_filtered::<&EntityInfoComponent, With<UnderConstructionComponent>>()
        .iter(world)
        .count()
}
