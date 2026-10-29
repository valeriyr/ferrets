//! Build order: workers raising construction sites, and sites that advance
//! themselves once a builder that does not join the crew has placed them.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    attack::Slain,
    build::BuilderAttendance,
    detection::Detection,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{FieldCoverage, FieldDecay, FieldDef, FieldPlacement, FieldVision},
    kinds::Kinds,
    location::Solidity,
    pool::Pool,
    pool_def::PoolId,
    registry::ContentRegistry,
    terrains::Terrains,
    transport::{PassengerConduct, PassengerFate},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_nav_grid::nav_grid::NavGrid;
use ferrets_simulation::{
    beside::Lean,
    command::PlayerCommand,
    components::{
        build::{BuildComponent, BuildStage},
        entity_info::EntityInfoComponent,
        hidden::Inside,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        rise::{RiseAttendance, RisingComponent},
    },
    entity_def::{self, Operation},
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent},
    game_loop::{
        damage,
        orders::{self, Refusal},
    },
    map::Map,
    movement_model::MovementModel,
    order::{AttackTarget, BuildTask, Order},
    resources::PlayerResources,
    session::{GameSession, player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn, supply,
};

//
// ─── Raising a site ───────────────────────────────────────────────────────────
//

#[test]
fn build_constructs_building() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
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
    // rising.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    {
        let world = app.world_mut();
        assert_eq!(utils::gold(world), 30);
        assert!(entity_def::hidden(world, worker));
        assert_eq!(rising(world), 1);
    }

    // Construction completes: the marker is gone and the worker reappears next to
    // the building.
    utils::run_ticks(&mut app, 6);
    assert!(!entity_def::hidden(app.world(), worker));
    let world = app.world_mut();
    assert_eq!(rising(world), 0);

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
fn builder_death_inside_site_keeps_price_spent() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
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

    // Wait until construction has started: the price is paid and the builder
    // works from inside the site.
    utils::run_ticks(&mut app, 12);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    // 80 granted − 50 for the depot.
    assert_eq!(utils::gold(app.world_mut()), 30);

    spawn::destroy_entity(app.world_mut(), worker);
    utils::run_ticks(&mut app, 6);

    // The site goes down with the builder that was raising it, and what it
    // cost goes with them: nothing pays a site back but a cancel aimed at it.
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 30);
}

#[test]
fn force_cancel_brings_hidden_builder_back_and_keeps_price_spent() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
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

    utils::force_cancel_orders(app.world_mut(), worker);

    // The order taken away destroys the unfinished building and the builder
    // reappears next to the site. Only CancelBuild pays a site back, so the 50
    // the depot cost stays spent.
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::hidden(app.world(), worker));
    assert_eq!(utils::gold(app.world_mut()), 30);
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
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
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
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 10, 10, 0);
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
    assert!(entity_def::hidden(app.world(), worker));

    // The site is destroyed under the builder working inside it.
    let site = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    spawn::destroy_entity(app.world_mut(), site);
    utils::run_ticks(&mut app, 2);

    assert!(
        !entity_def::hidden(app.world(), worker),
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
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 5, 5, 0);
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
        !entity_def::hidden(app.world(), mason),
        "a builder declared present raises the site from beside it instead of \
         vanishing into it"
    );

    // Still exposed for the whole job, then finishes normally.
    utils::run_ticks(&mut app, 6);
    assert!(!entity_def::hidden(app.world(), mason));
    assert_eq!(rising(app.world_mut()), 0, "construction completed");
}

#[test]
fn builder_walks_up_to_site_rather_than_into_it() {
    // The depot would span (10, 10) to (11, 11) and the mason approaches from the
    // east. Closing on the position alone would stop it a cell short of (10, 10) —
    // which is inside the footprint, where it would block its own site.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 14, 11, 0);
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
    let (surveyor, surveyor_id) = utils::create_owned(&mut app, "surveyor", 14, 10, 0);
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
fn builder_standing_on_site_steps_off_it_and_raises_it() {
    // A builder that works in the open stands on the cells its depot would
    // take, (10, 10) to (11, 11): it walks off the footprint to the nearest
    // cell it still builds from, (10, 9) straight up from its own, then raises
    // the site and pays for it.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 10, 10, 0);
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
    // 80 granted − the depot's 50.
    assert_eq!(utils::gold(app.world_mut()), 30);
    assert_eq!(utils::cell_of(app.world_mut(), mason), CellPos::new(10, 9));
    assert!(!entity_def::hidden(app.world(), mason));
}

#[test]
fn idle_ally_on_site_walks_off_and_site_goes_up() {
    // An idle soldier of the builder's own stands on (11, 11), a cell of the
    // depot's (10..11, 10..11). The mason, at (9, 10) on the fifth tick, asks
    // it off: it walks to (12, 11), the nearest cell clear of the site, and
    // the site goes up on the seventh tick, once its crossing has left the
    // site's cells.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 11, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    // 80 granted − the depot's 50.
    assert_eq!(utils::gold(app.world_mut()), 30);
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(12, 11)
    );
    assert_eq!(utils::cell_of(app.world_mut(), mason), CellPos::new(9, 10));
}

#[test]
fn idle_allies_on_site_walk_off_to_cells_of_their_own() {
    // Walls leave the mason's way in, the depot's (10..11, 10..11) and two
    // cells east of it, (12, 11) and (12, 12). Idle soldiers stand on (11, 10)
    // and (11, 11), and (12, 11) is the nearest free cell for both: asked
    // together on the sixth tick, the first is sent there and the second to
    // (12, 12), so neither drifts back onto the site, and the depot goes up
    // on the tenth tick, once both have stepped clear of it.
    let mut app = utils::orders_app();
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [
            (8, 10),
            (9, 10),
            (10, 10),
            (11, 10),
            (10, 11),
            (11, 11),
            (12, 11),
            (12, 12),
        ] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
        }
    }
    let (_, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (first, _) = utils::create_owned(&mut app, "soldier", 11, 10, 0);
    let (second, _) = utils::create_owned(&mut app, "soldier", 11, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::move_target(&app, first), Some(utils::pos(12, 11)));
    assert_eq!(utils::move_target(&app, second), Some(utils::pos(12, 12)));

    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::cell_of(app.world_mut(), first), CellPos::new(12, 11));
    assert_eq!(
        utils::cell_of(app.world_mut(), second),
        CellPos::new(12, 12)
    );
}

#[test]
fn stunned_ally_on_site_keeps_it_from_going_up() {
    // The own soldier idle on (11, 11) is stunned: an entity that does not
    // operate holds, so the mason, judging the ground on the sixth tick, gives
    // the order up at once rather than waiting on it.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 11, 11, 0);
    utils::grant_gold(&mut app, 80);
    utils::disable_for(&mut app, soldier, 60);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 5);
    assert!(!utils::order_queue_is_empty(app.world_mut(), mason));
    utils::run_ticks(&mut app, 1);

    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(11, 11)
    );
}

#[test]
fn teammates_idle_soldier_on_site_walks_off_and_site_goes_up() {
    // Players 0 and 1 share a team: the soldier of player 1 idle on
    // (11, 11) is asked off as the mason's own would be, walks to (12, 11),
    // and the site goes up on the seventh tick.
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
    ]);
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    let (_, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 11, 11, 1);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(12, 11)
    );
}

#[test]
fn enemy_on_site_keeps_it_from_going_up() {
    // The enemy soldier on (11, 11) holds: the mason, judging the ground on
    // the sixth tick, gives the order up at once.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 11, 11, 1);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 5);
    assert!(!utils::order_queue_is_empty(app.world_mut(), mason));
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(11, 11)
    );
}

#[test]
fn busy_ally_on_site_keeps_it_from_going_up() {
    // The soldier on (11, 11) is at work, striking an enemy wall beside it: an
    // ally with an order of its own holds the site as an enemy would, and the
    // mason, judging the ground on the sixth tick, gives the order up at once.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 8, 10, 0);
    let (soldier, soldier_id) = utils::create_owned(&mut app, "soldier", 11, 11, 0);
    let (_, wall_id) = utils::create_owned(&mut app, "rampart", 12, 11, 1);
    utils::grant_gold(&mut app, 80);
    utils::select(&mut app, soldier_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(wall_id),
            flush: true,
        },
    );

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 5);
    assert!(!utils::order_queue_is_empty(app.world_mut(), mason));
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert_eq!(
        utils::cell_of(app.world_mut(), soldier),
        CellPos::new(11, 11)
    );
}

#[test]
fn founding_waits_out_its_patience_on_ally_with_nowhere_to_go() {
    // Every cell but the site's and the mason's is walled: the idle soldier on
    // the site has nowhere to walk off to, and the founding waits for it,
    // then gives up.
    let mut app = utils::orders_app();
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [(9, 10), (10, 10), (11, 10), (10, 11), (11, 11)] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
        }
    }
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::create_owned(&mut app, "soldier", 11, 11, 0);
    utils::grant_gold(&mut app, 80);

    // The order is taken up on the third tick and judged every tick from
    // then on: forty ticks of patience, and the forty-first judgment, on the
    // forty-third tick, gives the order up.
    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 42);
    assert!(!utils::order_queue_is_empty(app.world_mut(), mason));
    utils::run_ticks(&mut app, 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
}

#[test]
fn site_has_one_founder() {
    // Two carpenters are sent to found one depot, the first standing on its
    // (10, 10). On the third tick the second, beside it, finds the first
    // standing on the ground at work on an order of its own: a busy body
    // holds, so the second founding ends there with nothing paid. The first
    // steps off on the fourth tick and raises the site alone on the sixth.
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 10, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 9, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, 3);
    assert!(utils::order_queue_is_empty(app.world_mut(), second));
    assert!(!utils::order_queue_is_empty(app.world_mut(), first));
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(crew_of_rising(app.world_mut()), vec![first_id]);
    // 80 granted − one depot's 50.
    assert_eq!(utils::gold(app.world_mut()), 30);
    assert_eq!(utils::cell_of(app.world_mut(), second), CellPos::new(9, 11));
}

#[test]
fn builder_standing_on_site_it_cannot_pay_for_gives_up_where_it_stands() {
    // Every gate is judged before the builder moves: with no gold for the
    // depot, the mason gives the order up on (10, 10) without stepping off.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 10, 10, 0);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::cell_of(app.world_mut(), mason), CellPos::new(10, 10));
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn builder_leaving_site_to_itself_steps_off_it_and_raises_it() {
    // The architect only places the depot, standing in the open on (10, 10)
    // inside it: it walks to (10, 9), straight up from its own cell, and the
    // site goes up behind it.
    let mut app = utils::orders_app();
    let (architect, architect_id) = utils::create_owned(&mut app, "architect", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        utils::cell_of(app.world_mut(), architect),
        CellPos::new(10, 9)
    );
}

#[test]
fn builder_lying_across_site_edge_steps_wholly_off_it() {
    // Under the continuous model the mason rests at (9.5, 10), across (9, 10)
    // and (10, 10), the depot's first cell: it walks onto a cell wholly off
    // the depot's (10..11, 10..11) instead of taking the half it already
    // stands on as arrived, and raises the site.
    let mut app = utils::continuous_orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    app.world_mut()
        .get_mut::<LocationComponent>(mason)
        .expect("the mason stands")
        .position = FixedUVec2::new(utils::fixed("9.5"), utils::fixed("10"));
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 20);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        entity_def::position(app.world(), mason),
        FixedUVec2::from(CellPos::new(9, 10))
    );
}

#[test]
fn builder_whose_site_ground_takes_with_it_there_raises_it_where_it_stands() {
    // A rug lies underfoot and is not kept off by a solid walker: the rug
    // layer raises it on (10, 10) under itself without stepping off.
    let mut app = builders_app();
    let (layer, layer_id) = utils::create_owned(&mut app, "rug_layer", 10, 10, 0);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: layer_id,
            type_name: "rug".into(),
            position: utils::pos(10, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "rug"), 1);
    assert_eq!(utils::cell_of(app.world_mut(), layer), CellPos::new(10, 10));
}

#[test]
fn builder_steps_off_its_site_wherever_it_walks_whatever_its_placement_rules() {
    // A warded mason may only be placed inside a ward, and no ward stands
    // anywhere: it still walks off the depot's (10..11, 10..11) to (10, 9),
    // straight up from its own cell, since a walk is held to no placement
    // rule, and raises the site.
    let mut app = builders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "warded_mason", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 12);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::cell_of(app.world_mut(), mason), CellPos::new(10, 9));
}

#[test]
fn consumed_builder_raises_site_it_stands_on_under_continuous_model() {
    // Under the continuous model a body's claim is rebuilt after the orders
    // run: the larva leaving the grid as it founds the depot on (10, 10) is
    // set aside for the site's own placement too.
    let mut app = utils::continuous_orders_app();
    let (_, larva_id) = utils::create_owned(&mut app, "larva", 10, 10, 0);
    utils::grant_gold(&mut app, 80);
    utils::run_ticks(&mut app, 1);

    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
}

#[test]
fn attached_builder_raises_site_it_stands_on_under_continuous_model() {
    let mut app = utils::continuous_orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 10, 10, 0);
    utils::grant_gold(&mut app, 80);
    utils::run_ticks(&mut app, 1);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(entity_def::attached(app.world(), roofer));
}

#[test]
fn builder_that_never_walks_standing_on_site_gives_up() {
    // A rooted mason works in the open on (10, 10), inside the depot it is
    // told to raise: it cannot step off, so the order is refused for want of
    // room, with nothing raised or paid.
    let mut app = builders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "rooted_mason", 10, 10, 0);
    utils::grant_gold(&mut app, 80);
    let order = Order::Build {
        task: BuildTask::found("depot", utils::pos(10, 10)),
    };
    assert_eq!(
        orders::can_start(app.world(), mason, &order),
        Err(Refusal::NoRoom)
    );

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn builder_that_never_walks_is_not_blamed_for_site_another_body_holds() {
    // A rooted mason on (10, 10) and a soldier on (11, 11), both inside the
    // 2×2 depot ordered at (10, 10): set aside, the mason would leave the
    // soldier keeping the site off, so the mason is not in its way and the
    // order may start; the ground refuses the site on arrival.
    let mut app = builders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "rooted_mason", 10, 10, 0);
    utils::create_owned(&mut app, "soldier", 11, 11, 0);
    utils::grant_gold(&mut app, 80);
    let order = Order::Build {
        task: BuildTask::found("depot", utils::pos(10, 10)),
    };
    assert_eq!(orders::can_start(app.world(), mason, &order), Ok(()));

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn wide_builder_standing_on_site_steps_off_it_and_raises_it() {
    // A 2×2 giant mason over (10..11, 10..11) and a depot ordered at (11, 10):
    // the giant walks to (9, 10), its footprint (9..10, 10..11) off the
    // depot's (11..12, 10..11) and beside it, and raises the site.
    let mut app = builders_app();
    let (giant, giant_id) = utils::create_owned(&mut app, "giant_mason", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: giant_id,
            type_name: "depot".into(),
            position: utils::pos(11, 10),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 20);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::cell_of(app.world_mut(), giant), CellPos::new(9, 10));
}

#[test]
fn builder_that_works_hidden_raises_site_it_stands_on() {
    // Leaving the map frees the cells, so a builder that disappears into its work can
    // raise a site from the spot it is standing on — the one thing the open worker
    // above cannot do.
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 10, 10, 0);
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
    assert!(entity_def::hidden(app.world(), worker));

    // Six ticks of work later it is back out beside what it raised.
    utils::run_ticks(&mut app, 6);
    assert_eq!(rising(app.world_mut()), 0);
    assert!(!entity_def::hidden(app.world(), worker));
}

#[test]
fn boxed_in_builder_finishes_site_and_waits_to_reappear() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 10, 10, 0);
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
    assert!(entity_def::hidden(app.world(), worker));

    // Take away every cell it could come back out onto, then let the work finish.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 6);

    // The walls are up regardless: the building is not held back by having nowhere
    // to put the builder. The builder waits off the map with a queued reveal, and
    // comes back onto the one cell that frees.
    assert_eq!(rising(app.world_mut()), 0, "the site finished on time");
    utils::assert_reveal_deferred_then_lands_on(&mut app, worker, CellPos::new(9, 9));
}

#[test]
fn queued_order_waits_while_boxed_in_builder_has_no_cell_to_return_to() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 10, 10, 0);
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
    utils::select(&mut app, worker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(14, 10),
            flush: false,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 6 + 3);

    // The site is done and the builder waits off the map; the walk queued
    // behind the build waits with it rather than starting from no cell.
    assert!(entity_def::hidden(app.world(), worker));
    assert!(entity_def::pending_reveal(app.world(), worker));
    assert!(
        !utils::order_queue_is_empty(app.world_mut(), worker),
        "the walk is still queued"
    );

    utils::set_all_cells_statically_occupied(app.world_mut(), false);
    utils::run_ticks(&mut app, 30);
    assert_eq!(utils::cell_of(app.world(), worker), CellPos::new(14, 10));
}

#[test]
fn seated_builder_is_left_to_its_order_when_site_is_destroyed() {
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(entity_def::attached(app.world(), roofer));
    let site = utils::owned_of_type(app.world_mut(), "depot", 0)[0];

    // The site's death sets down only workers hidden inside it: a seated one
    // is still in its berth as the site dies, and leaves through its own
    // order.
    spawn::destroy_entity(app.world_mut(), site);
    assert!(entity_def::attached(app.world(), roofer));
}

#[test]
fn boxed_in_attached_builder_goes_off_map_and_waits() {
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(entity_def::attached(app.world(), roofer));

    // No cell to step onto when the work ends: the roofer gives its berth
    // back and waits off the map with its return deferred, stranded like any
    // other body owed a cell; the depot is complete meanwhile.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 6);
    assert!(!entity_def::attached(app.world(), roofer));
    assert!(entity_def::hidden(app.world(), roofer));
    assert!(entity_def::pending_reveal(app.world(), roofer));
    assert_eq!(
        entity_def::operation(app.world(), roofer),
        Operation::Stranded
    );

    // The cell frees: the roofer comes back onto it.
    utils::free_cell(&mut app, CellPos::new(9, 9));
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::hidden(app.world(), roofer));
    assert!(!entity_def::pending_reveal(app.world(), roofer));
    assert_eq!(utils::cell_of(app.world(), roofer), CellPos::new(9, 9));
}

#[test]
fn destroyed_while_waiting_for_cell_dies_off_map() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 10, 10, 0);
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
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::run_ticks(&mut app, 6);
    assert!(entity_def::pending_reveal(app.world(), worker));

    // A dying entity never comes back: the deferred return is dropped at
    // once, and a cell freeing afterwards sets nothing down.
    spawn::destroy_entity(app.world_mut(), worker);
    assert!(!entity_def::pending_reveal(app.world(), worker));
    utils::free_cell(&mut app, CellPos::new(9, 9));
    utils::run_ticks(&mut app, 1);
    assert!(app.world().get::<LocationComponent>(worker).is_none());
    utils::run_ticks(&mut app, 2);
    utils::assert_despawned(app.world_mut(), worker);
}

#[test]
fn builder_faces_site_rather_than_corner_its_position_names() {
    let mut app = utils::orders_app();
    // Level with the depot's lower row and just east of it, so the building lies
    // due west. Its position names the north-west cell, which does not.
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 12, 11, 0);
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
    assert_eq!(
        facing,
        Facing::WEST,
        "it faces squarely west toward the depot: aiming at the position would \
         tilt it north"
    );
}

//
// ─── Sharing a site ─────────────────────────────────────────────────────────
//

#[test]
fn crew_shares_one_site_and_each_member_adds_own_work() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
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
    assert!(!entity_def::hidden(app.world(), first));
    assert!(!entity_def::hidden(app.world(), second));

    // Two builders on the site, two ticks of work a tick.
    let before = rising_progress(app.world_mut());
    utils::run_ticks(&mut app, 1);
    assert_eq!(rising_progress(app.world_mut()), before + 2);

    // A rise time of six is served by a pair in three ticks of work.
    utils::run_ticks(&mut app, 2);
    assert_eq!(
        rising(app.world_mut()),
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
    let (_, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(rising(app.world_mut()), 1);

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
            .map(|build| build.stage),
        Some(BuildStage::Raising(site_id)),
        "the second builder took up the site rather than trailing after it"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        30,
        "and joining costs nothing on top of what the site was paid for"
    );
}

#[test]
fn builder_sent_to_join_site_never_founds_one() {
    // The second carpenter is sent from (24, 10) to join the depot the first
    // raised; the site is called off while it walks there. Its order ends:
    // nothing new is raised, and the stockpile keeps the refund.
    let mut app = utils::orders_app();
    let (_, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 24, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    utils::run_ticks(&mut app, utils::APPLY);
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
    utils::run_ticks(&mut app, 1);
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: site_id });
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        entity_def::orders(app.world(), second)
            .last()
            .and_then(|order| order.build_task().cloned()),
        Some(BuildTask::join(site_id)),
        "the cancel lands while the joiner walks to the site"
    );
    utils::run_ticks(&mut app, 40);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert!(utils::order_queue_is_empty(app.world_mut(), second));
    // 80 granted − 50 for the site + 50 back on the cancel.
    assert_eq!(utils::gold(app.world_mut()), 80);
}

#[test]
fn joiner_whose_site_rises_before_it_arrives_ends_its_order() {
    // The second carpenter is sent from (30, 10) to join the depot the first
    // raised; the first finishes it alone on the sixth tick. The joiner
    // judges the site again only once its walk ends, beside it on (12, 11):
    // the site has risen, and its order ends with nothing raised or paid.
    let mut app = utils::orders_app();
    let (_, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 30, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    utils::run_ticks(&mut app, utils::APPLY);
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
    utils::run_ticks(&mut app, 6);
    assert!(!entity_def::rising(app.world(), site));
    assert_eq!(
        entity_def::orders(app.world(), second)
            .last()
            .and_then(|order| order.build_task().cloned()),
        Some(BuildTask::join(site_id)),
        "the site stands while the joiner walks to it"
    );
    utils::run_ticks(&mut app, 33);
    assert!(!utils::order_queue_is_empty(app.world_mut(), second));
    utils::run_ticks(&mut app, 1);

    assert!(utils::order_queue_is_empty(app.world_mut(), second));
    assert_eq!(
        utils::cell_of(app.world_mut(), second),
        CellPos::new(12, 11)
    );
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    // 80 granted − 50 for the one depot.
    assert_eq!(utils::gold(app.world_mut()), 30);
}

#[test]
fn worker_coming_out_of_finished_site_has_idle_allies_make_way() {
    // Walls leave the depot's site and a corridor east along row 10. The
    // worker founds from (12, 10) and works hidden inside; idle soldiers hold
    // (13, 10) to (19, 10), and another takes (12, 10) once the worker is in,
    // so when the depot stands on the ninth tick every cell the worker could
    // come out on within the search is held, (20, 10) lying beyond it. Waiting
    // off the map, it has the soldiers asked aside, and comes out on (19, 10)
    // on the eleventh tick.
    let mut app = utils::orders_app();
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    {
        let mut map = app.world_mut().resource_mut::<Map>();
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, y), false);
        }
        for x in 12..=30 {
            map.set_static_occupied(utils::GROUND, CellPos::new(x, 10), false);
        }
    }
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 12, 10, 0);
    for x in 13..=19 {
        utils::create_owned(&mut app, "soldier", x, 10, 0);
    }
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, worker_id);
    utils::run_ticks(&mut app, 4);
    assert!(entity_def::hidden(app.world(), worker));
    // The cell the worker went in from is taken once it is inside.
    utils::create_owned(&mut app, "soldier", 12, 10, 0);
    utils::run_ticks(&mut app, 6);
    assert_eq!(rising(app.world_mut()), 0);
    assert!(entity_def::hidden(app.world(), worker));
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::hidden(app.world(), worker));
    assert_eq!(utils::cell_of(app.world(), worker), CellPos::new(19, 10));
}

#[test]
fn builder_that_works_alone_turns_away_from_site_another_holds() {
    let mut app = utils::orders_app();
    let (_, first_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "mason", 12, 11, 0);
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
    let before = rising_progress(app.world_mut());
    utils::run_ticks(&mut app, 1);
    assert_eq!(rising_progress(app.world_mut()), before + 1);
}

#[test]
fn canceling_one_of_crew_leaves_site_standing() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (_, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(rising(app.world_mut()), 1);

    utils::force_cancel_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        rising(app.world_mut()),
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
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        crew_of_rising(app.world_mut()),
        vec![first_id, second_id],
        "both carpenters show up in the site's crew"
    );

    // One of the pair stops. The site keeps standing, and keeps the other builder.
    utils::force_cancel_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        crew_of_rising(app.world_mut()),
        vec![second_id],
        "one builder leaving does not empty the site"
    );

    // The last builder out empties the crew and, having worked in the open, leaves
    // the site standing halted rather than tearing it down.
    utils::force_cancel_orders(app.world_mut(), second);
    utils::run_ticks(&mut app, 1);
    assert!(crew_of_rising(app.world_mut()).is_empty());
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
}

//
// ─── Sites left standing ───────────────────────────────────────────────────────
//

#[test]
fn open_builder_ordered_away_leaves_site_halted_with_its_progress() {
    // A builder that stood beside its site leaves it standing when it goes,
    // work and payment intact: nothing is refunded, and the ticks it put in
    // are still there for whoever takes the site up next.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    // The order lands on tick 3 and the site goes up; two more ticks of work follow.
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(rising_progress(app.world_mut()), 2);
    assert_eq!(utils::gold(app.world_mut()), 30);

    utils::force_cancel_orders(app.world_mut(), mason);
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(
        rising_progress(app.world_mut()),
        2,
        "a halted site keeps its work"
    );
    assert_eq!(
        utils::gold(app.world_mut()),
        30,
        "nothing is refunded for a standing site"
    );
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn site_changing_hands_under_its_builder_is_left_halted_to_its_new_side() {
    // The mason's depot goes up on tick 3 and takes two ticks of work; then
    // it is handed to player 1. On its next tick the mason, no longer raising
    // its own side's site, steps off it, and the site stands halted with the
    // work in it.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(rising_progress(app.world_mut()), 2);

    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, mason_id, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
    assert!(crew_of_rising(app.world_mut()).is_empty());
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(rising_progress(app.world_mut()), 2);
    assert_eq!(entity_def::owner(app.world(), site), Some(1));
}

#[test]
fn hidden_worker_leaves_site_changing_hands_halted() {
    // The worker founds the depot and works it hidden inside from the twelfth
    // tick; the site is handed to player 1. On its next tick the worker comes
    // out on (9, 9), where it went in — walking from (5, 5) it stopped on the
    // diagonal by the depot's corner at (10, 10), within its build range of
    // one — still player 0's, its order ended, and the site stands halted for
    // player 1 rather than taken down as a hidden crew's would be.
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, worker_id);
    utils::run_ticks(&mut app, 12);
    assert!(entity_def::hidden(app.world(), worker));

    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, worker_id, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::hidden(app.world(), worker));
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
    assert_eq!(utils::cell_of(app.world_mut(), worker), CellPos::new(9, 9));
    assert_eq!(entity_def::owner(app.world(), worker), Some(0));
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(entity_def::owner(app.world(), site), Some(1));
}

#[test]
fn crew_of_two_leaves_site_changing_hands_halted_with_its_work() {
    // Two carpenters raise the depot together: four ticks of work by the
    // fifth tick. Handed to player 1, both orders end on the next tick, and
    // the last out leaves the site halted with the four ticks in it.
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(crew_of_rising(app.world_mut()), vec![first_id, second_id]);
    assert_eq!(rising_progress(app.world_mut()), 4);

    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, first_id, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    assert!(utils::order_queue_is_empty(app.world_mut(), first));
    assert!(utils::order_queue_is_empty(app.world_mut(), second));
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(rising_progress(app.world_mut()), 4);
}

#[test]
fn consumed_builder_is_captured_with_its_site_and_finishes_it() {
    // The larva works the depot hidden from the twelfth tick, at no progress
    // yet; the site is handed to player 1, and the larva goes with it, still
    // inside and on its order, each announced as captured. The depot rises
    // in six ticks of work, one a tick: five after five ticks, risen for
    // player 1 on the sixth, consuming the larva.
    let mut app = recording_orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    assert_eq!(rising_progress(app.world_mut()), 0);

    let site = depot(app.world_mut());
    let site_id = entity_def::simulation_id(app.world(), site);
    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);
    assert_eq!(entity_def::owner(app.world(), larva), Some(1));
    assert!(entity_def::hidden(app.world(), larva));

    utils::run_ticks(&mut app, 5);
    assert_eq!(
        captures(&app),
        vec![(site_id, Some(0), 1), (larva_id, Some(0), 1)]
    );
    assert_eq!(rising_progress(app.world_mut()), 5);
    utils::run_ticks(&mut app, 1);

    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(entity_def::owner(app.world(), site), Some(1));
    assert_eq!(deaths(&app), vec![(larva_id, DeathCause::Consumed)]);
}

#[test]
fn captors_cancel_sets_consumed_builder_down_as_its_own() {
    // The larva's site is handed to player 1 with the larva inside; player 1
    // calls the site off, and the larva comes back onto the map as player 1's.
    let mut app = utils::orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    let site = depot(app.world_mut());
    let site_id = entity_def::simulation_id(app.world(), site);
    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);

    // Player 1's cancel, carried on this tick's frame: three ticks see the
    // site gone and the larva set down, the fewest that do.
    let at = app.world().resource::<GameSession>().tick();
    utils::run_ticks_commanding(
        &mut app,
        3,
        1,
        at,
        vec![PlayerCommand::CancelBuild { site: site_id }],
    );

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert!(!entity_def::hidden(app.world(), larva));
    // Back on (9, 9), where it went in.
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(9, 9));
    assert_eq!(entity_def::owner(app.world(), larva), Some(1));
}

#[test]
fn consumed_builder_captured_with_its_site_keeps_only_its_build() {
    // Player 0 queues a move behind the larva's build; handed to player 1
    // with the site, the larva keeps only the build, and once player 1 calls
    // the site off it stays where it is set down rather than walk on player
    // 0's way.
    let mut app = utils::orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::select(&mut app, larva_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 20),
            flush: false,
        },
    );
    utils::run_ticks(&mut app, 12);
    assert!(matches!(
        entity_def::orders(app.world(), larva).as_slice(),
        [Order::Build { .. }, Order::Move { .. }]
    ));
    let site = depot(app.world_mut());
    let site_id = entity_def::simulation_id(app.world(), site);

    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);
    assert!(matches!(
        entity_def::orders(app.world(), larva).as_slice(),
        [Order::Build { .. }]
    ));

    // Player 1's cancel, carried on this tick's frame: three ticks see the
    // site gone and the larva set down, the fewest that do.
    let at = app.world().resource::<GameSession>().tick();
    utils::run_ticks_commanding(
        &mut app,
        3,
        1,
        at,
        vec![PlayerCommand::CancelBuild { site: site_id }],
    );
    assert!(utils::order_queue_is_empty(app.world_mut(), larva));
    // Back on (9, 9), where it went in, not on its way to (20, 20).
    assert_eq!(utils::cell_of(app.world(), larva), CellPos::new(9, 9));
}

#[test]
fn consumed_builder_dying_inside_captured_site_takes_it_down() {
    // The larva is struck dead inside its depot on the tick the site is
    // handed to player 1, so it is not handed over with it; its death ends
    // its build, and the site it was to become comes down with it, whatever
    // side the site is now.
    let mut app = recording_orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    let site = depot(app.world_mut());
    let site_id = entity_def::simulation_id(app.world(), site);

    damage::apply(
        app.world_mut(),
        site_id,
        larva,
        FixedU64::from_num(1000),
        Slain::Nothing,
    );
    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);
    assert_eq!(entity_def::owner(app.world(), larva), Some(0));
    utils::run_ticks(&mut app, 1);

    // Struck down before the tick, the larva's death ends its build on it,
    // taking the site down.
    assert_eq!(
        deaths(&app),
        vec![
            (
                larva_id,
                DeathCause::Killed {
                    by: site_id,
                    by_owner: Some(0)
                }
            ),
            (site_id, DeathCause::Canceled)
        ]
    );
}

#[test]
fn consumed_holder_captured_with_its_site_sets_down_passengers_captor_bars() {
    // The ferrying larva carries player 0's rider into the depot it becomes;
    // handed to player 1 with the site, it boards its owner's side only, so
    // the rider is set down beside where the larva went in.
    let mut app = carrying_larvae_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "ferrying_larva", 5, 5, 0);
    let (rider, _) = carried_rider(&mut app, larva, larva_id);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    assert!(entity_def::hidden(app.world(), larva));

    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);

    assert_eq!(entity_def::owner(app.world(), larva), Some(1));
    assert_eq!(utils::passengers_of(app.world(), larva), []);
    assert!(!entity_def::hidden(app.world(), rider));
    // Beside (9, 9), where the larva went in, within its unload range of one:
    // (10, 9).
    assert_eq!(utils::cell_of(app.world(), rider), CellPos::new(10, 9));
    assert_eq!(entity_def::owner(app.world(), rider), Some(0));
}

#[test]
fn consumed_holder_captured_with_its_site_loses_passengers_captor_bars() {
    // The devouring larva's fate for a passenger its hold no longer admits is
    // death, as the capture of any holder would have it.
    let mut app = carrying_larvae_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "devouring_larva", 5, 5, 0);
    let (_, rider_id) = carried_rider(&mut app, larva, larva_id);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    assert!(entity_def::hidden(app.world(), larva));

    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, larva_id, Lean::Nothing);
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::passengers_of(app.world(), larva), []);
    assert_eq!(
        deaths(&app),
        vec![(rider_id, DeathCause::PassengerBarred { holder: larva_id })]
    );
}

#[test]
fn hidden_worker_canceled_on_site_changing_hands_leaves_it_halted() {
    // The worker's order is taken away on the tick its site is handed to
    // player 1, so it leaves through the cancel rather than its work: the
    // last out of a site no longer its side's leaves it halted, as the work
    // would, not taken down.
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    order_depot(&mut app, worker_id);
    utils::run_ticks(&mut app, 12);
    let site = depot(app.world_mut());
    spawn::change_owner(app.world_mut(), site, 1, worker_id, Lean::Nothing);
    utils::force_cancel_orders(app.world_mut(), worker);

    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::hidden(app.world(), worker));
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
}

#[test]
fn halted_site_is_taken_up_by_next_builder_and_finished() {
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    let (_, carpenter_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::force_cancel_orders(app.world_mut(), mason);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );

    // The carpenter takes the halted site up as a crew of one — nobody is paid
    // again — and its four remaining ticks of work finish it.
    order_depot(&mut app, carpenter_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(crew_of_rising(app.world_mut()), vec![carpenter_id]);
    assert_eq!(utils::gold(app.world_mut()), 30);
    utils::run_ticks(&mut app, 4);
    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
}

#[test]
fn builder_that_leaves_sites_to_themselves_takes_up_halted_one_on_its_own_terms() {
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    let (architect, architect_id) = utils::create_owned(&mut app, "architect", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::force_cancel_orders(app.world_mut(), mason);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );

    // The architect leaves a site it raises to itself, so it leaves this one to
    // itself too rather than joining a crew it is not built to be on — and its
    // order is done the moment it has taken the site up.
    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(matches!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Unattended { .. })
    ));
    assert!(utils::order_queue_is_empty(app.world_mut(), architect));
    assert_eq!(utils::gold(app.world_mut()), 30, "nobody paid twice");

    // It advances itself from there: four ticks were still owed.
    utils::run_ticks(&mut app, 4);
    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
}

#[test]
fn attached_builder_stands_on_site_holding_no_cells() {
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);

    // On the site — its footprint cell nearest to where the roofer stood — and
    // on the map: visible to targeting, never hidden.
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(entity_def::attached(app.world(), roofer));
    assert!(!entity_def::hidden(app.world(), roofer));
    assert_eq!(utils::cell_of(app.world(), roofer), CellPos::new(10, 10));
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .interactable(app.world(), roofer_id)
            .is_some(),
        "an attached builder can be targeted"
    );
    // The cell it left is nobody's now.
    let grid = app.world().resource::<Map>().nav_grid();
    assert!(!grid.is_claimed_by(utils::GROUND, CellPos::new(9, 10)));

    // Six ticks of work later it steps back out onto a free cell beside the depot.
    utils::run_ticks(&mut app, 6);
    assert_eq!(rising(app.world_mut()), 0);
    assert!(!entity_def::attached(app.world(), roofer));
    let depot = depot(app.world_mut());
    utils::assert_adjacent_to_footprint(app.world_mut(), roofer, depot);
    let cell = utils::cell_of(app.world(), roofer);
    let grid = app.world().resource::<Map>().nav_grid();
    assert!(
        grid.is_claimed_by(utils::GROUND, cell),
        "back on the grid, it holds its cell"
    );
}

#[test]
fn attached_builder_standing_on_site_raises_it_without_stepping_off() {
    // The roofer stands on (10, 10), inside the depot's footprint: seated in
    // its berths once the site stands, it holds none of the site's cells, so
    // the site goes up at once and it takes the berth on the cell it stood on.
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(entity_def::attached(app.world(), roofer));
    assert_eq!(utils::cell_of(app.world(), roofer), CellPos::new(10, 10));
}

#[test]
fn passable_attached_builder_raises_site_it_stands_on() {
    // A passable builder holds its ground against static footprints though
    // it claims no cells; set aside as it takes a berth, it raises the depot
    // from (10, 10) inside it and sits on the cell it stood on.
    let mut app = builders_app();
    let (wisp, wisp_id) = utils::create_owned(&mut app, "wisp_roofer", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, wisp_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(entity_def::attached(app.world(), wisp));
    assert_eq!(utils::cell_of(app.world(), wisp), CellPos::new(10, 10));
}

#[test]
fn attached_builder_that_never_walks_raises_site_it_stands_on() {
    // A rooted roofer marks its cell as a standing footprint: it leaves the
    // grid before the depot is stamped over it, and the depot holds all four
    // of its cells, (10..11, 10..11).
    let mut app = builders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "rooted_roofer", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(entity_def::attached(app.world(), roofer));
    let grid = app.world().resource::<Map>().nav_grid();
    for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        assert!(
            grid.is_statically_occupied_by(utils::GROUND, CellPos::new(x, y)),
            "({x}, {y})"
        );
    }
}

#[test]
fn attached_builders_take_distinct_berths_of_site() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "roofer", 12, 12, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(crew_of_rising(app.world_mut()), vec![first_id, second_id]);
    // Each in the rim berth nearest to where it arrived: the north-west cell
    // for the first, the south-east one for the second.
    assert_eq!(utils::cell_of(app.world(), first), CellPos::new(10, 10));
    assert_eq!(utils::cell_of(app.world(), second), CellPos::new(11, 11));
}

#[test]
fn attached_builder_shot_dead_on_its_site_leaves_it_halted() {
    // The roofer sits on the depot's south-east berth, in the open; an enemy
    // soldier standing beside it cuts it down while it works, and the site
    // stands halted with the work put in so far.
    let mut app = recording_orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 12, 11, 0);
    let (_, soldier_id) = utils::create_owned(&mut app, "soldier", 12, 12, 1);
    utils::grant_gold(&mut app, 80);

    let soldier = app
        .world()
        .resource::<EntityIndex>()
        .alive(soldier_id)
        .unwrap();
    app.world_mut()
        .get_mut::<OrderQueueComponent>(soldier)
        .unwrap()
        .push(
            Order::Attack {
                target: AttackTarget::Entity(roofer_id),
                leash: None,
            },
            Some(CancelPolicy::Force),
        );
    order_depot(&mut app, roofer_id);

    // The build lands on the third tick and the roofer takes the berth at
    // (11.5, 11.5), a step from the soldier. Twenty health, ten a hit, a hit
    // two ticks into each four-tick swing: struck on the third tick and the
    // seventh, dead on the seventh — after that tick's work, since hits land
    // once the orders have run — with four ticks of work put in.
    utils::run_ticks(&mut app, 7);
    assert!(
        deaths(&app)
            .iter()
            .any(|&(id, cause)| id == roofer_id && matches!(cause, DeathCause::Killed { .. })),
        "the roofer was killed on its site"
    );
    utils::run_ticks(&mut app, 3);
    utils::assert_despawned(app.world_mut(), roofer);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(rising_progress(app.world_mut()), 4);
}

#[test]
fn attached_builder_destroyed_at_work_leaves_site_halted() {
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(rising_progress(app.world_mut()), 2);

    spawn::destroy_entity(app.world_mut(), roofer);
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), roofer);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        rising_attendance(app.world_mut()),
        Some(RiseAttendance::Halted)
    );
    assert_eq!(rising_progress(app.world_mut()), 2);
}

//
// ─── Canceling a site ─────────────────────────────────────────────────────────
//

#[test]
fn cancel_build_tears_down_site_and_refunds_it() {
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let depot = depot(app.world_mut());
    let site = entity_def::simulation_id(app.world(), depot);

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    // The command lands on the third tick; the site dies over its two-tick dying
    // phase, and the mason's order ends on the next tick, its site gone.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(
        utils::gold(app.world_mut()),
        80,
        "the whole price comes back"
    );
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn canceled_captured_site_refunds_its_payer() {
    let mut app = utils::orders_app();
    // Player 1's depot, rising, handed to player 0, who calls it off.
    let (depot, depot_id) =
        utils::create_rising(&mut app, "depot", 10, 10, 1, RiseAttendance::Halted);
    spawn::change_owner(app.world_mut(), depot, 0, depot_id, Lean::Nothing);
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: depot_id });
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(!utils::alive(&app, depot_id));
    // The price, 50, goes back to player 1, who paid it, not to the player who canceled it.
    assert_eq!(
        app.world().resource::<PlayerResources>().amount(1, "gold"),
        50
    );
    assert_eq!(utils::gold(app.world()), 0);
}

#[test]
fn cancel_build_brings_hidden_builder_back_out() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, worker_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(entity_def::hidden(app.world(), worker));
    let depot = depot(app.world_mut());
    let site = entity_def::simulation_id(app.world(), depot);

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert!(!entity_def::hidden(app.world(), worker));
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn cancel_build_ignores_rival_site() {
    let mut app = utils::orders_app();
    // A site of the other player's, raised straight onto the map.
    let (depot, site) = utils::create_rising(&mut app, "depot", 14, 14, 1, RiseAttendance::Halted);
    app.world_mut()
        .entity_mut(depot)
        .get_mut::<RisingComponent>()
        .expect("the depot was just created rising")
        .progress = 4;

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        app.world()
            .get::<RisingComponent>(depot)
            .map(|rising| rising.progress),
        Some(4),
        "a player cancels only its own sites"
    );
}

#[test]
fn cancel_build_ignores_finished_building() {
    let mut app = utils::orders_app();
    let (_, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 6);
    assert_eq!(rising(app.world_mut()), 0);
    let depot = depot(app.world_mut());
    let site = entity_def::simulation_id(app.world(), depot);

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(utils::gold(app.world_mut()), 30);
}

//
// ─── Unattended and consumed builders ─────────────────────────────────────────
//

#[test]
fn unattended_builder_places_site_and_leaves_it_to_advance_itself() {
    let mut app = recording_orders_app();
    let (architect, architect_id) = utils::create_owned(&mut app, "architect", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, architect_id);

    // The architect walks up, pays, and is done: the site stands with nobody on
    // it, naming its founder.
    utils::run_ticks(&mut app, 12);
    {
        let world = app.world_mut();
        assert_eq!(utils::count_of_type(world, "depot"), 1);
        assert_eq!(utils::gold(world), 30);
        assert_eq!(rising(world), 1);
        assert!(crew_of_rising(world).is_empty());
        assert_eq!(founder_of_rising(world), Some(architect_id));
        assert!(!entity_def::hidden(world, architect));
        assert!(utils::order_queue_is_empty(world, architect));
    }

    // The work goes on without it, one tick at a time, and the completion is
    // announced with the founder as its builder.
    utils::run_ticks(&mut app, 8);
    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(completed_by(&app), vec![architect_id]);
}

#[test]
fn unattended_site_outlives_its_founder() {
    let mut app = recording_orders_app();
    let (architect, architect_id) = utils::create_owned(&mut app, "architect", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, 12);
    assert_eq!(rising(app.world_mut()), 1);

    spawn::destroy_entity(app.world_mut(), architect);

    // The founder is gone; the site finishes regardless and still names it.
    utils::run_ticks(&mut app, 10);
    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(completed_by(&app), vec![architect_id]);
}

#[test]
fn nobody_joins_unattended_site() {
    let mut app = utils::orders_app();
    let (_, architect_id) = utils::create_owned(&mut app, "architect", 5, 5, 0);
    // Already within reach of the site, so its order resolves at once.
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, 12);
    let progress_before = rising_progress(app.world_mut());
    // The architect walks four cells at half a cell per tick once the command
    // lands after the three-tick delay, so the site is raised on the eleventh
    // tick and has one tick of progress by the twelfth.
    assert_eq!(progress_before, 1);

    // A crew builder sent to the same site has no work to take up: it finishes
    // its order and the site keeps its own pace.
    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 3);
    let world = app.world_mut();
    assert!(crew_of_rising(world).is_empty());
    assert_eq!(rising_progress(world), progress_before + 3);
    assert!(utils::order_queue_is_empty(world, mason));
}

#[test]
fn consumed_builder_works_site_hidden_and_is_consumed_when_it_completes() {
    let mut app = recording_orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, larva_id);

    // The larva walks up, pays, and steps inside: its own build order works the
    // site as a crew of one, and it eats no supply while inside.
    utils::run_ticks(&mut app, 12);
    {
        let world = app.world_mut();
        assert_eq!(utils::count_of_type(world, "depot"), 1);
        assert_eq!(utils::gold(world), 30);
        assert_eq!(rising(world), 1);
        assert!(entity_def::hidden(world, larva));
        assert!(!utils::order_queue_is_empty(world, larva));
        assert_eq!(crew_of_rising(world), vec![larva_id]);
        assert_eq!(supply::used(world, 0), FixedU64::ZERO);
    }
    assert!(deaths(&app).is_empty());

    // The site completes and consumes the larva: its death is announced as
    // consumed rather than as a loss, and the completion names it.
    utils::run_ticks(&mut app, 6);
    assert_eq!(rising(app.world_mut()), 0);
    assert_eq!(completed_by(&app), vec![larva_id]);
    assert_eq!(deaths(&app), vec![(larva_id, DeathCause::Consumed)]);
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), larva);
}

#[test]
fn force_cancel_brings_consumed_builder_back_and_keeps_price_spent() {
    let mut app = recording_orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, larva_id);
    utils::run_ticks(&mut app, 12);
    assert_eq!(rising(app.world_mut()), 1);
    assert_eq!(supply::used(app.world(), 0), FixedU64::ZERO);

    let site = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    let site_id = entity_def::simulation_id(app.world(), site);
    utils::force_cancel_orders(app.world_mut(), larva);
    // The cancel lands next tick; the abandoned site then dies over its two
    // ticks and is removed.
    utils::run_ticks(&mut app, 6);

    // The site is abandoned and the larva stands beside where it was, alive
    // and counted again. Nothing is paid back: 80 granted − 50 for the depot.
    let world = app.world_mut();
    assert_eq!(utils::count_of_type(world, "depot"), 0);
    assert_eq!(utils::gold(world), 30);
    assert!(!entity_def::hidden(world, larva));
    assert_eq!(supply::used(world, 0), FixedU64::ONE);
    assert_eq!(
        deaths(&app),
        vec![(site_id, DeathCause::Canceled)],
        "the site is the one death, and the builder was not spent"
    );
}

#[test]
fn consumed_builder_survives_placement_that_fails() {
    let mut app = recording_orders_app();
    let (larva, larva_id) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    // An enemy soldier standing in the footprint blocks the site.
    utils::create_owned(&mut app, "soldier", 10, 10, 1);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, larva_id);

    // Nothing was placed, so nothing was paid and nobody was consumed: the larva
    // is back on the map beside where the site would have stood.
    utils::run_ticks(&mut app, 14);
    let world = app.world_mut();
    assert_eq!(utils::count_of_type(world, "depot"), 0);
    assert_eq!(utils::gold(world), 80);
    assert!(!entity_def::hidden(world, larva));
    assert!(deaths(&app).is_empty());
}

//
// ─── Solidity and placement ───────────────────────────────────────────────────
//

#[test]
fn static_footprint_is_not_raised_over_what_stands_underfoot() {
    let mut app = solidity_app();
    assert!(place(&mut app, "mole", 10, 10).is_some());

    // The mole claims no cell, so the grid is clear, and it is still there:
    // a depot is refused on it and raised one cell over.
    assert!(
        place(&mut app, "depot", 10, 10).is_none(),
        "the ground is held"
    );
    assert!(place(&mut app, "depot", 12, 10).is_some());
}

#[test]
fn static_footprint_is_not_raised_over_passable_body() {
    let mut app = solidity_app();
    assert!(place(&mut app, "pebble", 10, 10).is_some());

    // The pebble claims no cell and is still there: a depot of another
    // solidity is refused on it and raised one cell over.
    assert!(place(&mut app, "depot", 10, 10).is_none());
    assert!(place(&mut app, "depot", 12, 10).is_some());
}

#[test]
fn passable_bodies_share_their_cells() {
    let mut app = solidity_app();
    assert!(place(&mut app, "pebble", 10, 10).is_some());

    assert!(place(&mut app, "pebble", 10, 10).is_some());
}

#[test]
fn mover_crosses_what_stands_underfoot() {
    let mut app = solidity_app();
    assert!(place(&mut app, "mole", 10, 10).is_some());

    // What movers do is the grid's business, and the mole claims nothing
    // there: a walker stands on the same cell.
    assert!(place(&mut app, "worker", 10, 10).is_some());
}

#[test]
fn static_footprint_is_raised_over_what_stands_underfoot_on_another_layer() {
    let mut app = solidity_app();
    assert!(place(&mut app, "kite", 10, 10).is_some());

    // The kite holds the air and the depot claims the ground, so they share no
    // layer and the kite refuses nothing.
    assert!(
        place(&mut app, "depot", 10, 10).is_some(),
        "an underfoot body refuses only a footprint that shares its layers"
    );
}

#[test]
fn static_footprint_is_raised_over_underfoot_body_that_is_off_map() {
    let mut app = solidity_app();
    let mole = place(&mut app, "mole", 10, 10).expect("the mole stands");
    // Off the map — aboard, inside a site or down a mine — it holds no ground.
    spawn::hide_entity(app.world_mut(), mole, Inside::Nothing);

    assert!(
        place(&mut app, "depot", 10, 10).is_some(),
        "what is off the map holds no ground"
    );
}

#[test]
fn building_is_refused_on_terrain_its_domain_does_not_take() {
    // A dock, the depot in a domain that takes land alone, its 2×2 footprint
    // at (10,10) over the one water cell (11,11): it goes up nowhere, and the
    // 50 gold it costs stays unspent.
    let mut app = utils::dry_orders_app(MovementModel::Cell, &[(11, 11)], |registry, dry| {
        let mut dock = registry
            .entity("depot")
            .expect("the orders content declares a depot")
            .clone();
        dock.name = "dock".to_string();
        registry.register(utils::in_domain(dock, dry));
        let mut wright = registry
            .entity("worker")
            .expect("the orders content declares a worker")
            .clone()
            .with_builder(
                ["dock"],
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            );
        wright.name = "wright".to_string();
        registry.register(wright);
    });
    let (wright, wright_id) = utils::create_owned(&mut app, "wright", 5, 5, 0);
    utils::grant_gold(&mut app, 80);
    let build_at = |app: &mut App, x: u32, y: u32| {
        utils::push_command(
            app,
            PlayerCommand::BuildEntity {
                builder: wright_id,
                type_name: "dock".into(),
                position: utils::pos(x, y),
                flush: true,
            },
        );
        utils::run_ticks(app, 12);
    };

    build_at(&mut app, 10, 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "dock"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert!(utils::order_queue_is_empty(app.world_mut(), wright));

    // One cell clear of the water, (12,10)..(13,11) all land, it goes up:
    // 80 − 50 = 30 gold left.
    build_at(&mut app, 12, 10);
    assert_eq!(utils::count_of_type(app.world_mut(), "dock"), 1);
    assert_eq!(utils::gold(app.world_mut()), 30);
}

//
// ─── Builders that never walk ─────────────────────────────────────────────────
//

#[test]
fn builder_that_never_walks_works_site_in_range_from_where_it_stands() {
    // A crew tower on (4, 10) raises a depot at (24, 10), 20 cells off and
    // within its range of 30: it never moves, and the depot rises in its 6
    // ticks.
    let mut app = builders_app();
    let (tower, tower_id) = utils::create_owned(&mut app, "crew_tower", 4, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_far_depot(&mut app, tower_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let depot = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    assert!(entity_def::rising(app.world(), depot));
    assert_eq!(utils::cell_of(app.world(), tower), CellPos::new(4, 10));
    utils::run_ticks(&mut app, 6);

    assert!(!entity_def::rising(app.world(), depot));
    assert_eq!(utils::cell_of(app.world(), tower), CellPos::new(4, 10));
    assert!(entity_def::stands_on_grid(app.world(), tower));
}

#[test]
fn builder_that_never_walks_is_refused_site_beyond_its_range() {
    // A rooted mason on (4, 10) builds within 1 cell: a depot at (24, 10) is
    // 20 cells off, so the order is refused as it is issued, nothing raised
    // or paid.
    let mut app = builders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "rooted_mason", 4, 10, 0);
    utils::grant_gold(&mut app, 80);
    let order = Order::Build {
        task: BuildTask::found("depot", utils::pos(24, 10)),
    };
    assert_eq!(
        orders::can_start(app.world(), mason, &order),
        Err(Refusal::OutOfReach)
    );

    order_far_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert_eq!(utils::gold(app.world_mut()), 80);
    assert!(utils::order_queue_is_empty(app.world_mut(), mason));
}

#[test]
fn unattended_builder_that_never_walks_founds_site_in_range_and_is_done() {
    // A founding tower on (4, 10) founds the depot at (24, 10) and leaves it
    // to itself, its order done the tick the site stands.
    let mut app = builders_app();
    let (tower, tower_id) = utils::create_owned(&mut app, "founding_tower", 4, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_far_depot(&mut app, tower_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert!(utils::order_queue_is_empty(app.world_mut(), tower));
    assert_eq!(utils::cell_of(app.world(), tower), CellPos::new(4, 10));
}

#[test]
fn hidden_builder_that_never_walks_comes_out_beside_site_in_range() {
    // A sheltering tower on (4, 10) goes into the depot at (24, 10), freeing
    // its cell, and comes out as the depot stands on the free cell beside it
    // nearest where it went in, (23, 10).
    let mut app = builders_app();
    let (tower, tower_id) = utils::create_owned(&mut app, "sheltering_tower", 4, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_far_depot(&mut app, tower_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), tower));
    assert!(
        !app.world()
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(4, 10))
    );
    utils::run_ticks(&mut app, 6);

    let depot = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    assert!(!entity_def::rising(app.world(), depot));
    assert!(!entity_def::hidden(app.world(), tower));
    assert_eq!(utils::cell_of(app.world(), tower), CellPos::new(23, 10));
}

#[test]
fn attached_builder_that_never_walks_takes_berth_of_site_in_range() {
    // A roofing tower on (4, 10) leaves its cell for the depot's berth over
    // (24, 10), and as the depot stands comes out on (24, 9), a free cell
    // beside that berth.
    let mut app = builders_app();
    let (tower, tower_id) = utils::create_owned(&mut app, "roofing_tower", 4, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_far_depot(&mut app, tower_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::attached(app.world(), tower));
    assert!(
        !app.world()
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, CellPos::new(4, 10))
    );
    utils::run_ticks(&mut app, 6);

    let depot = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    assert!(!entity_def::rising(app.world(), depot));
    assert!(!entity_def::attached(app.world(), tower));
    assert_eq!(utils::cell_of(app.world(), tower), CellPos::new(24, 9));
}

#[test]
fn consumed_builder_that_never_walks_is_spent_on_site_in_range() {
    // A spent tower on (4, 10) disappears into the depot at (24, 10) and is
    // despawned as it stands.
    let mut app = builders_app();
    let (tower, tower_id) = utils::create_owned(&mut app, "spent_tower", 4, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_far_depot(&mut app, tower_id);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert!(entity_def::hidden(app.world(), tower));
    utils::run_ticks(&mut app, 6);

    let depot = utils::single_owned_of_type(app.world_mut(), "depot", 0);
    assert!(!entity_def::rising(app.world(), depot));
    assert!(!utils::alive(&app, tower_id));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Creates an entity of `type_name` at `(x, y)` for player 0, or `None` when
/// the ground refuses it.
fn place(app: &mut App, type_name: &str, x: u32, y: u32) -> Option<Entity> {
    utils::create_entity(app.world_mut(), type_name, utils::pos(x, y), Some(0))
        .map(|(entity, _)| entity)
}

/// The orders roster plus five builders of depots: a `wisp_roofer`, a
/// passable walker taking a berth of the site; a `rooted_roofer`, one that
/// never walks; a 2×2 `giant_mason` working in the open; a `rooted_mason`,
/// working in the open and never walking; and a `warded_mason` placed only
/// inside a `ward` field nothing spreads. Plus a `rug_layer` raising `rug`s,
/// underfoot sites, and five 1×1 builders of depots that never walk, with a
/// build range of 30: a `crew_tower` working in the open, a `founding_tower`
/// leaving its sites to themselves, a `sheltering_tower` working inside, a
/// `roofing_tower` taking a berth, and a `spent_tower` consumed by its site.
fn builders_app() -> App {
    utils::dry_orders_app(MovementModel::Cell, &[], |registry, _| {
        let roofing = || {
            BuilderAttendance::Crew(WorkPresence::Attached(Attachment::new(
                "rim",
                BerthStance::Still,
            )))
        };
        let walking = |def: EntityTypeDef, size: u32| {
            def.with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5") * FixedU64::from_num(size),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            )
        };
        let builder = |name: &str, size: u32, solidity| {
            EntityTypeDef::new(name)
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(size, size),
                    solidity,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
        };
        registry.register(
            walking(builder("wisp_roofer", 1, Solidity::Passable), 1)
                .with_builder(["depot"], roofing()),
        );
        registry.register(
            builder("rooted_roofer", 1, Solidity::Solid).with_builder(["depot"], roofing()),
        );
        let ward = registry.register_field(
            "ward",
            FieldDef::new(
                Terrains::Any,
                utils::GROUND.into(),
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            walking(builder("warded_mason", 1, Solidity::Solid), 1)
                .with_field_placement([FieldPlacement::Requires {
                    field: ward,
                    of: Affiliation::Anyone,
                    coverage: FieldCoverage::Every,
                }])
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
        registry.register(
            EntityTypeDef::new("rug")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Underfoot,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_rise(6, utils::rise_initial_pools(&[PoolId::HEALTH])),
        );
        registry.register(
            walking(builder("rug_layer", 1, Solidity::Solid), 1).with_builder(
                ["rug"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            ),
        );
        registry.register(builder("rooted_mason", 1, Solidity::Solid).with_builder(
            ["depot"],
            BuilderAttendance::Crew(WorkPresence::Present {
                crew: CrewLimit::ONE,
            }),
        ));
        registry.register(
            walking(builder("giant_mason", 2, Solidity::Solid), 2).with_builder(
                ["depot"],
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            ),
        );
        for (name, attendance) in [
            (
                "crew_tower",
                BuilderAttendance::Crew(WorkPresence::Present {
                    crew: CrewLimit::ONE,
                }),
            ),
            ("founding_tower", BuilderAttendance::Unattended),
            (
                "sheltering_tower",
                BuilderAttendance::Crew(WorkPresence::Hidden {
                    crew: CrewLimit::ONE,
                }),
            ),
            ("roofing_tower", roofing()),
            ("spent_tower", BuilderAttendance::Consumed),
        ] {
            registry.register(
                builder(name, 1, Solidity::Solid)
                    .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(30))
                    .with_builder(["depot"], attendance),
            );
        }
    })
}

/// Orders `builder` to raise a depot at (24, 10), far from the others.
fn order_far_depot(app: &mut App, builder: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::BuildEntity {
            builder,
            type_name: "depot".into(),
            position: utils::pos(24, 10),
            flush: true,
        },
    );
}

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

/// The orders roster on a two-layer map, with a `mole` that stands underfoot
/// and a `pebble` that is passable, both one cell of ground, and a
/// `kite` that stands underfoot on the air layer. Two humans, session started.
fn solidity_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    // Two layers, so a body that holds one of them can be shown not to refuse
    // a footprint on the other.
    {
        let mut grid = NavGrid::new(32, 32, |_| utils::LAND);
        grid.add_flat_layer(utils::GROUND, 1);
        grid.add_flat_layer(utils::AIR, 2);
        ferrets_bevy_plugin::map::install_map(
            app.world_mut(),
            Map::new(
                "test",
                Projection::Isometric,
                MovementModel::Cell,
                grid,
                vec![],
            ),
        );
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        utils::register_air(&mut registry);
        registry.register(
            EntityTypeDef::new("mole")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Underfoot,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("pebble")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Passable,
                    utils::GROUND,
                )
                .with_pool(Pool::health(20)),
        );
        // The same underfoot body a layer up, to show that holding ground on
        // one layer says nothing about another.
        registry.register(
            EntityTypeDef::new("kite")
                .with_location(
                    utils::AIR_DOMAIN,
                    CellSize::ONE,
                    Solidity::Underfoot,
                    utils::AIR,
                )
                .with_pool(Pool::health(20)),
        );
    }
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    app
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
            utils::walker("surveyor", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::from_num(3))
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
    }
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    app
}

/// Ticks of work put into the rising entities on the map.
fn rising_progress(world: &mut World) -> u32 {
    world
        .query::<&RisingComponent>()
        .iter(world)
        .map(|rising| rising.progress)
        .sum()
}

/// The crew on the one rising entity the sharing suite raises, in [`SimulationId`] order.
fn crew_of_rising(world: &mut World) -> Vec<SimulationId> {
    world
        .query::<&RisingComponent>()
        .iter(world)
        .flat_map(|rising| match &rising.attendance {
            RiseAttendance::Crew { workers } => workers.iter().copied().collect::<Vec<_>>(),
            RiseAttendance::Unattended { .. } | RiseAttendance::Halted => Vec::new(),
        })
        .collect()
}

/// How the one rising entity on the map is attended, if the map holds one.
fn rising_attendance(world: &mut World) -> Option<RiseAttendance> {
    world
        .query::<&RisingComponent>()
        .iter(world)
        .map(|rising| rising.attendance.clone())
        .next()
}

/// The one depot on the map.
fn depot(world: &mut World) -> Entity {
    utils::single_owned_of_type(world, "depot", 0)
}

/// The founder of the one unattended rising entity, if the map holds one.
fn founder_of_rising(world: &mut World) -> Option<SimulationId> {
    world
        .query::<&RisingComponent>()
        .iter(world)
        .find_map(|rising| match rising.attendance {
            RiseAttendance::Crew { .. } | RiseAttendance::Halted => None,
            RiseAttendance::Unattended { founder } => Some(founder),
        })
}

/// The orders roster with a `rider` that boards, and two larvae consumed into
/// the depot they raise that carry riders of their owner's side: a
/// `ferrying_larva` setting the ones it bars down, a `devouring_larva`
/// destroying them. Two humans, session started, every tick's announcements
/// kept.
fn carrying_larvae_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            utils::walker("rider", utils::GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
        );
        for (name, fate) in [
            ("ferrying_larva", PassengerFate::Eject),
            ("devouring_larva", PassengerFate::Destroy),
        ] {
            registry.register(
                utils::walker(name, utils::GROUND_DOMAIN)
                    .with_pool(Pool::health(20))
                    .with_dying(2, [])
                    .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                    .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::ONE)
                    .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
                    .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                    .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ONE)
                    .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ONE)
                    .with_builder(["depot"], BuilderAttendance::Consumed)
                    .with_transporter(
                        Kinds::types(["rider"]),
                        Affiliation::Own,
                        fate,
                        PassengerConduct::Shelter,
                    ),
            );
        }
    }
    utils::register_orders_content(&mut app);
    utils::start_game(&mut app);
    utils::record_announcements(&mut app);
    app
}

/// A rider of player 0 created beside `larva` and sent aboard it.
fn carried_rider(app: &mut App, larva: Entity, larva_id: SimulationId) -> (Entity, SimulationId) {
    let rider = utils::create_owned(app, "rider", 6, 5, 0);
    utils::send_to(app, rider.1, larva_id);
    utils::run_until_aboard(app, larva, 1, 20);
    rider
}

/// The economy content roster with every tick's announcements kept, for the
/// suite that asserts who a completion names.
fn recording_orders_app() -> App {
    let mut app = utils::orders_app();
    utils::record_announcements(&mut app);
    app
}

/// Every death announced so far, as the subject and why.
fn deaths(app: &App) -> Vec<(SimulationId, DeathCause)> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityDied { entity, cause, .. } => Some((*entity, *cause)),
            _ => None,
        })
        .collect()
}

/// Every capture announced so far, as the subject, the side it was taken
/// from and the side it went to.
fn captures(app: &App) -> Vec<(SimulationId, Option<PlayerId>, PlayerId)> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityCaptured {
                entity, from, to, ..
            } => Some((*entity, *from, *to)),
            _ => None,
        })
        .collect()
}

/// Whoever finished every rise so far.
fn completed_by(app: &App) -> Vec<SimulationId> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::EntityRisen { by, .. } => Some(*by),
            _ => None,
        })
        .collect()
}

/// How many entities are still rising.
fn rising(world: &mut World) -> usize {
    world
        .query_filtered::<&EntityInfoComponent, With<RisingComponent>>()
        .iter(world)
        .count()
}
