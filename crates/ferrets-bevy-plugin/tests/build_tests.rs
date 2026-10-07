//! Build order: workers raising construction sites, and sites that advance
//! themselves once a builder that does not join the crew has placed them.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    build::{BuilderAttendance, RiseStart, SitePool},
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    pool_shift::PoolShift,
    registry::ContentRegistry,
    requirement::Requirement,
    stack_rule::StackRule,
    work::{CrewLimit, WorkPresence},
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{FixedU64, facing::Facing};
use ferrets_pathfinder::{mover_shape::MoverShape, nav_grid::NavGrid};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        attached::AttachedComponent,
        build::{self, BuildComponent, SiteWork, UnderConstructionComponent},
        entity_info::EntityInfoComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        pools,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent},
    map::Map,
    movement_model::MovementModel,
    order::{AttackTarget, Order},
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn, supply,
};

#[test]
fn site_never_bears_built_passive_tick_it_is_founded() {
    let mut app = utils::orders_app();
    let (_, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "kiosk".into(),
            position: utils::pos(8, 8),
            flush: true,
        },
    );

    // Tick by tick until the site stands: founded a site, it is never fitted
    // `opened`, which holds only once built.
    let site = (0..30)
        .find_map(|_| {
            utils::run_ticks(&mut app, 1);
            (utils::count_of_type(app.world_mut(), "kiosk") == 1)
                .then(|| utils::single_owned_of_type(app.world_mut(), "kiosk", 0))
        })
        .expect("the worker founds the kiosk");
    let opened = app
        .world()
        .resource::<ContentRegistry>()
        .entity_buff("opened")
        .expect("the fixture registers opened");
    assert!(!entity_def::bears(app.world(), site, opened));
}

#[test]
fn rising_site_gains_health_with_work() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    // A quarter of 400 at founding.
    assert_eq!(utils::health_as_u32(&app, rampart), 100);

    // (400 − 100) / 10 = 30 a tick of work, moved as it is put in:
    // 100 + 5 × 30 = 250.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);

    // The tenth tick completes it, full.
    utils::run_ticks(&mut app, 5);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(rampart)
            .is_none()
    );
    assert_eq!(utils::health_as_u32(&app, rampart), 400);
}

#[test]
fn damage_taken_while_rising_stays_as_deficit() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 5);
    // 250, wounded by 100 to 150; the line goes on to 400: 400 − 100 = 300.
    utils::wound(&mut app, rampart, "100");
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 300);
}

#[test]
fn site_raised_maximum_mid_build_still_finishes_full() {
    // Whatever shift the raise names, the line moves the site, not the shift.
    // The clamp arm reads the same with the shift applied, since a clamp
    // leaves a value under a raised maximum where it is.
    for (name, pool_shift) in [
        ("fortified_by_clamp", PoolShift::Clamp),
        ("fortified_by_difference", PoolShift::Difference),
        ("fortified_by_share", PoolShift::Share),
    ] {
        let mut app = utils::orders_app();
        let fortified = register_health_shift(&mut app, name, "100", pool_shift);
        let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
        // Five ticks of work: 100 + 5 × 30 = 250.
        utils::run_ticks(&mut app, 5);
        assert_eq!(utils::health_as_u32(&app, rampart), 250, "{name}");

        // The maximum rises to 500. The sixth tick's work: 125 + 375 × 6 / 10
        // = 350; at ten, full.
        utils::apply_buff(app.world_mut(), rampart, fortified);
        utils::run_ticks(&mut app, 1);
        assert_eq!(utils::health_as_u32(&app, rampart), 350, "{name}");
        utils::run_ticks(&mut app, 4);
        assert_eq!(utils::health_as_u32(&app, rampart), 500, "{name}");
    }
}

#[test]
fn finished_site_gives_its_pool_back_to_fold() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Difference);
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health_as_u32(&app, rampart), 400);

    // Built, its health follows its maximum by the raise's own shift again:
    // 400 + 100 = 500.
    utils::apply_buff(app.world_mut(), rampart, fortified);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, rampart), 500);
}

#[test]
fn halted_site_does_not_rise() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    // Five ticks of work: 100 + 5 × 30 = 250.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);

    // Halted, no work goes in and the line stands.
    halt(&mut app, rampart);
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);
}

#[test]
fn site_lowered_maximum_mid_build_still_finishes_full() {
    // Whatever shift the cut names, the line moves the site, not the shift.
    for (name, pool_shift) in [
        ("weakened_by_clamp", PoolShift::Clamp),
        ("weakened_by_difference", PoolShift::Difference),
        ("weakened_by_share", PoolShift::Share),
    ] {
        let mut app = utils::orders_app();
        let weakened = register_health_shift(&mut app, name, "-200", pool_shift);
        let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
        // Five ticks of work: 100 + 5 × 30 = 250.
        utils::run_ticks(&mut app, 5);
        assert_eq!(utils::health_as_u32(&app, rampart), 250, "{name}");

        // The maximum falls to 200. The sixth tick's work: 50 + 150 × 6 / 10
        // = 140; at ten, full.
        utils::apply_buff(app.world_mut(), rampart, weakened);
        utils::run_ticks(&mut app, 1);
        assert_eq!(utils::health_as_u32(&app, rampart), 140, "{name}");
        utils::run_ticks(&mut app, 4);
        assert_eq!(utils::health_as_u32(&app, rampart), 200, "{name}");
    }
}

#[test]
fn falling_line_leaves_live_site_its_last_sliver() {
    let mut app = utils::orders_app();
    let weakened = register_health_shift(&mut app, "weakened", "-200", PoolShift::Clamp);
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 5);
    halt(&mut app, rampart);
    // 250 wounded by 200 to 50; under the lowered maximum the line falls from
    // 250 to 50 + 150 × 5 / 10 = 125, 75 more than the 50 left, which leaves
    // the live site the smallest value.
    utils::wound(&mut app, rampart, "200");
    utils::apply_buff(app.world_mut(), rampart, weakened);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, rampart), FixedU64::DELTA);
    assert_eq!(utils::count_of_type(app.world_mut(), "rampart"), 1);
}

#[test]
fn withheld_pool_comes_with_finished_building_at_its_initial() {
    let mut app = utils::orders_app();
    let (tower, _) = utils::create_unattended_site(&mut app, "flare_tower", 8, 8, 0);
    assert_eq!(
        entity_def::pool_value(app.world(), tower, PoolId::ENERGY),
        None
    );
    // The fourth tick completes it, gaining its energy at a quarter of 60,
    // 15, which the next fold settles where it stands.
    utils::run_ticks(&mut app, 4);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(tower)
            .is_none()
    );
    assert_eq!(utils::energy_as_u32(&app, tower), 15);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, tower), 15);
}

#[test]
fn rising_pool_keeps_what_it_regenerates_besides() {
    let mut app = utils::orders_app();
    let (cistern, _) = utils::create_unattended_site(&mut app, "cistern", 8, 8, 0);
    // Half of 80 at founding.
    assert_eq!(utils::energy_as_u32(&app, cistern), 40);

    // (80 − 40) / 4 = 10 a tick of work, and a point regenerated each tick:
    // 40 + 3 × 10 + 3 × 1 = 73.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy_as_u32(&app, cistern), 73);

    // The fourth tick completes it: 73 + 10 + 1 = 84, held under 80.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, cistern), 80);
}

#[test]
fn site_founded_by_build_order_rises_to_full() {
    let mut app = utils::orders_app();
    let (_, worker_id) = utils::create_owned(&mut app, "worker", 5, 5, 0);
    utils::grant_gold(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: worker_id,
            type_name: "rampart".into(),
            position: utils::pos(8, 8),
            flush: true,
        },
    );
    while utils::count_of_type(app.world_mut(), "rampart") == 0 {
        utils::run_ticks(&mut app, 1);
    }
    let rampart = utils::single_owned_of_type(app.world_mut(), "rampart", 0);
    let mut seen = Vec::new();
    for _ in 0..12 {
        seen.push((
            progress_of(&app, rampart),
            utils::health_as_u32(&app, rampart),
        ));
        utils::run_ticks(&mut app, 1);
    }
    // Founded at a quarter of 400, then 30 a tick of work as it is put in:
    // 100 + 30 × progress, full once built.
    let mut expected: Vec<(Option<u32>, u32)> = (0..10)
        .map(|progress| (Some(progress), 100 + 30 * progress))
        .collect();
    expected.extend([(None, 400), (None, 400)]);
    assert_eq!(seen, expected);
}

#[test]
fn rising_site_starts_under_maximum_its_passives_leave() {
    let mut app = utils::orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        // While it is a site, a braced rampart's maximum is 200 higher.
        let braced = registry.register_entity_buff(
            "braced",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, "200")],
                    pool_shift: PoolShift::Clamp,
                })],
                lasting: Lasting::While(Requirement::Unless(Box::new(Requirement::Built))),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("braced_rampart")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(400))
                .with_dying(2, [])
                .with_passives([braced])
                .with_build(
                    10,
                    [(
                        PoolId::HEALTH,
                        SitePool::Rising(RiseStart::Share(utils::fixed("0.25"))),
                    )],
                ),
        );
    }
    let (rampart, _) = utils::create_unattended_site(&mut app, "braced_rampart", 8, 8, 0);
    // A quarter of 400 + 200, before any tick.
    assert_eq!(utils::health_as_u32(&app, rampart), 150);

    // Five ticks of work: 150 + 450 × 5 / 10.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 375);
}

#[test]
fn site_holds_pool_from_its_own_initial() {
    let mut app = utils::orders_app();
    app.world_mut().resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("lamp_post")
            .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
            .with_pool(Pool::health(100))
            .with_pool(Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(60),
                FixedU64::ZERO,
                FixedU64::ZERO,
                PoolInitial::Share(utils::fixed("0.25")),
            ))
            .with_dying(2, [])
            .with_build(4, utils::site_initial(&[PoolId::HEALTH, PoolId::ENERGY])),
    );
    let (post, _) = utils::create_crewed_site(&mut app, "lamp_post", 8, 8, 0);
    // A quarter of 60.
    assert_eq!(utils::energy_as_u32(&app, post), 15);
}

#[test]
fn built_pools_settle_under_built_passives_at_next_fold() {
    let mut app = utils::orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        // Once built, a keep's health and energy maxima are higher.
        let finished = registry.register_entity_buff(
            "finished",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![
                        utils::flat(EntityStatId::MAX_HEALTH, "100"),
                        utils::flat(EntityStatId::MAX_ENERGY, "60"),
                    ],
                    pool_shift: PoolShift::Clamp,
                })],
                lasting: Lasting::While(Requirement::Built),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("keep_wall")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(400))
                .with_pool(Pool::builtin(
                    PoolId::ENERGY,
                    FixedU64::from_num(60),
                    FixedU64::ZERO,
                    FixedU64::ZERO,
                    PoolInitial::Share(utils::fixed("0.25")),
                ))
                .with_dying(2, [])
                .with_passives([finished])
                .with_build(
                    10,
                    [
                        (
                            PoolId::HEALTH,
                            SitePool::Rising(RiseStart::Share(utils::fixed("0.25"))),
                        ),
                        (PoolId::ENERGY, SitePool::Withheld),
                    ],
                ),
        );
    }
    let (keep, _) = utils::create_unattended_site(&mut app, "keep_wall", 8, 8, 0);
    utils::run_ticks(&mut app, 10);
    assert!(
        app.world()
            .get::<UnderConstructionComponent>(keep)
            .is_none()
    );
    // Completed: its line reached the site's 400, and its energy stands at a
    // quarter of 60, 15, until the next fold settles it.
    assert_eq!(utils::health_as_u32(&app, keep), 400);
    assert_eq!(utils::energy_as_u32(&app, keep), 15);

    // Settled at the next fold, under the maxima `finished` gives: full
    // health, 400 + 100, and energy at a quarter of 60 + 60.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 500);
    assert_eq!(utils::energy_as_u32(&app, keep), 30);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 500);
    assert_eq!(utils::energy_as_u32(&app, keep), 30);
}

#[test]
fn settling_keeps_what_withheld_pool_lost_since_completion() {
    let mut app = utils::orders_app();
    let (tower, _) = utils::create_unattended_site(&mut app, "flare_tower", 8, 8, 0);
    utils::run_ticks(&mut app, 4);
    // Completed at a quarter of 60, 15, then 5 spent before the settle.
    pools::drain(app.world_mut(), tower, PoolId::ENERGY, utils::fixed("5"));
    utils::run_ticks(&mut app, 1);
    // Settled from its line of 15 to its initial of 15: 15 − 5 = 10 stands.
    assert_eq!(utils::energy_as_u32(&app, tower), 10);
}

#[test]
fn two_builders_raise_line_with_each_tick_of_work() {
    let mut app = utils::orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let mut joiner = registry
            .entity("carpenter")
            .expect("the fixture registers a carpenter")
            .clone();
        joiner.name = "joiner".to_string();
        registry.register(joiner.with_builder(
            ["rampart"],
            BuilderAttendance::Crew(WorkPresence::Present {
                crew: CrewLimit::Unlimited,
            }),
        ));
    }
    let (_, first) = utils::create_owned(&mut app, "joiner", 9, 10, 0);
    let (_, second) = utils::create_owned(&mut app, "joiner", 12, 11, 0);
    utils::grant_gold(&mut app, 80);
    for builder in [first, second] {
        utils::push_command(
            &mut app,
            PlayerCommand::BuildEntity {
                builder,
                type_name: "rampart".into(),
                position: utils::pos(10, 10),
                flush: true,
            },
        );
    }
    utils::run_ticks(&mut app, utils::APPLY);
    let rampart = utils::single_owned_of_type(app.world_mut(), "rampart", 0);
    let mut seen = Vec::new();
    for _ in 0..8 {
        seen.push((
            progress_of(&app, rampart),
            utils::health_as_u32(&app, rampart),
        ));
        utils::run_ticks(&mut app, 1);
    }
    // 30 a tick of work, as it is put in: one builder arrives first, then
    // both put in a tick each — 100 + 30 × progress, full at completion.
    assert_eq!(
        seen,
        vec![
            (Some(0), 100),
            (Some(1), 130),
            (Some(2), 160),
            (Some(3), 190),
            (Some(5), 250),
            (Some(7), 310),
            (Some(9), 370),
            (None, 400),
        ]
    );
}

#[test]
fn halted_site_lines_up_under_raised_maximum() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Difference);
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);
    halt(&mut app, rampart);

    // No work goes in, but the maximum rises to 500: the line stands again
    // at 125 + 375 × 5 / 10 = 312.5, and stays there.
    utils::apply_buff(app.world_mut(), rampart, fortified);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, rampart), utils::fixed("312.5"));
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::health(&app, rampart), utils::fixed("312.5"));
}

#[test]
#[should_panic(expected = "a site's work stops at its build time: 11 > 10")]
fn rising_past_build_time_panics() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    app.world_mut()
        .get_mut::<UnderConstructionComponent>(rampart)
        .expect("a site carries its construction state")
        .progress = 11;
    build::rise(app.world_mut(), rampart);
}

#[test]
fn buff_on_completing_tick_moves_rising_pool_by_its_line() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Clamp);
    let (rampart, _) = utils::create_unattended_site(&mut app, "rampart", 8, 8, 0);
    // The tenth tick completes it at 400; wounded to 350 and buffed before
    // the next fold, its pool still follows the line.
    utils::run_ticks(&mut app, 10);
    utils::wound(&mut app, rampart, "50");
    utils::apply_buff(app.world_mut(), rampart, fortified);
    utils::run_ticks(&mut app, 1);
    // Settled by the line, 400 → 500, keeping the 50 deficit: 450 of 500,
    // where the clamp alone would leave a finished building at 350 of 500.
    assert_eq!(utils::health_as_u32(&app, rampart), 450);
}

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
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
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
fn builder_standing_on_site_blocks_it_and_is_never_moved() {
    // A builder that works in the open is left alone, so its own cells block the
    // footprint exactly as anything else standing there would. The order gives up
    // rather than shoving it out of the way.
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 10, 10, 0);
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
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Six ticks of work later it is back out beside what it raised.
    utils::run_ticks(&mut app, 6);
    assert_eq!(under_construction(app.world_mut()), 0);
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_none());
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
    assert!(app.world_mut().get::<HiddenComponent>(worker).is_some());

    // Take away every cell it could come back out onto, then let the work finish.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
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
    let (_, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
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
    let before = site_progress(app.world_mut());
    utils::run_ticks(&mut app, 1);
    assert_eq!(site_progress(app.world_mut()), before + 1);
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
    assert_eq!(under_construction(app.world_mut()), 1);

    utils::force_cancel_orders(app.world_mut(), first);
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
    let (first, first_id) = utils::create_owned(&mut app, "carpenter", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
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
    utils::force_cancel_orders(app.world_mut(), first);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        crew_of_site(app.world_mut()),
        vec![second_id],
        "one builder leaving does not empty the site"
    );

    // The last builder out empties the crew and, having worked in the open, leaves
    // the site standing halted rather than tearing it down.
    utils::force_cancel_orders(app.world_mut(), second);
    utils::run_ticks(&mut app, 1);
    assert!(crew_of_site(app.world_mut()).is_empty());
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));
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
    assert_eq!(site_progress(app.world_mut()), 2);
    assert_eq!(utils::gold(app.world_mut()), 30);

    utils::force_cancel_orders(app.world_mut(), mason);
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));
    assert_eq!(
        site_progress(app.world_mut()),
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
fn halted_site_is_taken_up_by_next_builder_and_finished() {
    let mut app = utils::orders_app();
    let (mason, mason_id) = utils::create_owned(&mut app, "mason", 9, 10, 0);
    let (_, carpenter_id) = utils::create_owned(&mut app, "carpenter", 12, 11, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    utils::force_cancel_orders(app.world_mut(), mason);
    utils::run_ticks(&mut app, 1);
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));

    // The carpenter takes the halted site up as a crew of one — nobody is paid
    // again — and its four remaining ticks of work finish it.
    order_depot(&mut app, carpenter_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(crew_of_site(app.world_mut()), vec![carpenter_id]);
    assert_eq!(utils::gold(app.world_mut()), 30);
    utils::run_ticks(&mut app, 4);
    assert_eq!(under_construction(app.world_mut()), 0);
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
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));

    // The architect leaves a site it raises to itself, so it leaves this one to
    // itself too rather than joining a crew it is not built to be on — and its
    // order is done the moment it has taken the site up.
    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(matches!(
        site_work(app.world_mut()),
        Some(SiteWork::Unattended { .. })
    ));
    assert!(utils::order_queue_is_empty(app.world_mut(), architect));
    assert_eq!(utils::gold(app.world_mut()), 30, "nobody paid twice");

    // It advances itself from there: four ticks were still owed.
    utils::run_ticks(&mut app, 4);
    assert_eq!(under_construction(app.world_mut()), 0);
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
    assert!(app.world().get::<AttachedComponent>(roofer).is_some());
    assert!(app.world().get::<HiddenComponent>(roofer).is_none());
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
    assert_eq!(under_construction(app.world_mut()), 0);
    assert!(app.world().get::<AttachedComponent>(roofer).is_none());
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
fn attached_builders_take_distinct_berths_of_site() {
    let mut app = utils::orders_app();
    let (first, first_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    let (second, second_id) = utils::create_owned(&mut app, "roofer", 12, 12, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, first_id);
    order_depot(&mut app, second_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(crew_of_site(app.world_mut()), vec![first_id, second_id]);
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
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));
    assert_eq!(site_progress(app.world_mut()), 4);
}

#[test]
fn attached_builder_killed_at_work_leaves_site_halted() {
    let mut app = utils::orders_app();
    let (roofer, roofer_id) = utils::create_owned(&mut app, "roofer", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, roofer_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(site_progress(app.world_mut()), 2);

    spawn::destroy_entity(app.world_mut(), roofer);
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), roofer);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(site_work(app.world_mut()), Some(SiteWork::Halted));
    assert_eq!(site_progress(app.world_mut()), 2);
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
fn cancel_build_brings_hidden_builder_back_out() {
    let mut app = utils::orders_app();
    let (worker, worker_id) = utils::create_owned(&mut app, "worker", 9, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, worker_id);
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(app.world().get::<HiddenComponent>(worker).is_some());
    let depot = depot(app.world_mut());
    let site = entity_def::simulation_id(app.world(), depot);

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 0);
    assert!(app.world().get::<HiddenComponent>(worker).is_none());
    assert!(utils::order_queue_is_empty(app.world_mut(), worker));
}

#[test]
fn cancel_build_ignores_rival_site() {
    let mut app = utils::orders_app();
    // A site of the other player's, raised straight onto the map.
    let (depot, site) = utils::create_site(&mut app, "depot", 14, 14, 1, SiteWork::Halted);
    app.world_mut()
        .entity_mut(depot)
        .get_mut::<UnderConstructionComponent>()
        .expect("the depot was just founded a site")
        .progress = 4;

    utils::push_command(&mut app, PlayerCommand::CancelBuild { site });
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "depot"), 1);
    assert_eq!(
        app.world()
            .get::<UnderConstructionComponent>(depot)
            .map(|site| site.progress),
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
    assert_eq!(under_construction(app.world_mut()), 0);
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
        assert_eq!(under_construction(world), 1);
        assert!(crew_of_site(world).is_empty());
        assert_eq!(founder_of_site(world), Some(architect_id));
        assert!(world.get::<HiddenComponent>(architect).is_none());
        assert!(utils::order_queue_is_empty(world, architect));
    }

    // The work goes on without it, one tick at a time, and the completion is
    // announced with the founder as its builder.
    utils::run_ticks(&mut app, 8);
    assert_eq!(under_construction(app.world_mut()), 0);
    assert_eq!(completed_by(&app), vec![architect_id]);
}

#[test]
fn unattended_site_outlives_its_founder() {
    let mut app = recording_orders_app();
    let (architect, architect_id) = utils::create_owned(&mut app, "architect", 5, 5, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, architect_id);
    utils::run_ticks(&mut app, 12);
    assert_eq!(under_construction(app.world_mut()), 1);

    spawn::destroy_entity(app.world_mut(), architect);

    // The founder is gone; the site finishes regardless and still names it.
    utils::run_ticks(&mut app, 10);
    assert_eq!(under_construction(app.world_mut()), 0);
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
    let progress_before = site_progress(app.world_mut());
    // The architect walks four cells at half a cell per tick once the command
    // lands after the three-tick delay, so the site is raised on the eleventh
    // tick and has one tick of progress by the twelfth.
    assert_eq!(progress_before, 1);

    // A crew builder sent to the same site has no work to take up: it finishes
    // its order and the site keeps its own pace.
    order_depot(&mut app, mason_id);
    utils::run_ticks(&mut app, 3);
    let world = app.world_mut();
    assert!(crew_of_site(world).is_empty());
    assert_eq!(site_progress(world), progress_before + 3);
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
        assert_eq!(under_construction(world), 1);
        assert!(world.get::<HiddenComponent>(larva).is_some());
        assert!(!utils::order_queue_is_empty(world, larva));
        assert_eq!(crew_of_site(world), vec![larva_id]);
        assert_eq!(supply::used(world, 0), FixedU64::ZERO);
    }
    assert!(deaths(&app).is_empty());

    // The site completes and consumes the larva: its death is announced as
    // consumed rather than as a loss, and the completion names it.
    utils::run_ticks(&mut app, 6);
    assert_eq!(under_construction(app.world_mut()), 0);
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
    assert_eq!(under_construction(app.world_mut()), 1);
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
    assert!(world.get::<HiddenComponent>(larva).is_none());
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
    // A soldier standing in the footprint blocks the site.
    utils::create_owned(&mut app, "soldier", 10, 10, 0);
    utils::grant_gold(&mut app, 80);

    order_depot(&mut app, larva_id);

    // Nothing was placed, so nothing was paid and nobody was consumed: the larva
    // is back on the map beside where the site would have stood.
    utils::run_ticks(&mut app, 14);
    let world = app.world_mut();
    assert_eq!(utils::count_of_type(world, "depot"), 0);
    assert_eq!(utils::gold(world), 80);
    assert!(world.get::<HiddenComponent>(larva).is_none());
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
fn static_footprint_is_raised_over_what_holds_nothing() {
    let mut app = solidity_app();
    assert!(place(&mut app, "pebble", 10, 10).is_some());

    // A fully passable body yields its ground: the depot goes up over it.
    assert!(place(&mut app, "depot", 10, 10).is_some());
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
    // Off the map — aboard, inside a site or down a mine — it holds no ground,
    // and the position it still carries is stale.
    app.world_mut().entity_mut(mole).insert(HiddenComponent);

    assert!(
        place(&mut app, "depot", 10, 10).is_some(),
        "what is off the map holds no ground"
    );
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
/// and a `pebble` that holds nothing at all, both one cell of ground, and a
/// `kite` that stands underfoot on the air layer. Two humans, session started.
fn solidity_app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    // Two layers, so a body that holds one of them can be shown not to refuse
    // a footprint on the other.
    {
        let mut grid = NavGrid::new(32, 32);
        grid.add_layer(utils::GROUND);
        grid.add_layer(utils::AIR);
        ferrets_bevy_plugin::map::install_map(
            app.world_mut(),
            Map::new(
                "test",
                Projection::Isometric,
                MovementModel::Cell,
                grid,
                vec![],
                &[MoverShape::point(utils::GROUND)],
            ),
        );
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(utils::AIR_LAYER), utils::AIR);
        registry.register(
            EntityTypeDef::new("mole")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Underfoot)
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("pebble")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_pool(Pool::health(20)),
        );
        // The same underfoot body a layer up, to show that holding ground on
        // one layer says nothing about another.
        registry.register(
            EntityTypeDef::new("kite")
                .with_location(utils::AIR, CellSize::ONE, Solidity::Underfoot)
                .with_pool(Pool::health(20)),
        );
    }
    utils::register_orders_content(&mut app);
    app.world_mut().resource_mut::<GameSession>().start();
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
            utils::walker("surveyor", utils::GROUND)
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
        .flat_map(|site| match &site.work {
            SiteWork::Crew { builders } => builders.iter().copied().collect::<Vec<_>>(),
            SiteWork::Unattended { .. } | SiteWork::Halted => Vec::new(),
        })
        .collect()
}

/// How the one site on the map is worked, if the map holds one.
fn site_work(world: &mut World) -> Option<SiteWork> {
    world
        .query::<&UnderConstructionComponent>()
        .iter(world)
        .map(|site| site.work.clone())
        .next()
}

/// The one depot on the map.
fn depot(world: &mut World) -> Entity {
    utils::single_owned_of_type(world, "depot", 0)
}

/// The founder of the one unattended site, if the map holds one.
fn founder_of_site(world: &mut World) -> Option<SimulationId> {
    world
        .query::<&UnderConstructionComponent>()
        .iter(world)
        .find_map(|site| match site.work {
            SiteWork::Crew { .. } | SiteWork::Halted => None,
            SiteWork::Unattended { founder } => Some(founder),
        })
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

/// The builder named by every construction completed so far.
fn completed_by(app: &App) -> Vec<SimulationId> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter_map(|event| match event {
            SimulationEvent::ConstructionCompleted { builder, .. } => Some(*builder),
            _ => None,
        })
        .collect()
}

/// How many sites are still going up.
fn under_construction(world: &mut World) -> usize {
    world
        .query_filtered::<&EntityInfoComponent, With<UnderConstructionComponent>>()
        .iter(world)
        .count()
}

/// Registers a lasting buff named `name` moving max health by `amount`
/// under `pool_shift`.
fn register_health_shift(
    app: &mut App,
    name: &str,
    amount: &str,
    pool_shift: PoolShift,
) -> EntityBuffId {
    app.world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            name,
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, amount)],
                    pool_shift,
                })],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        )
}

/// Halts the site `site`: no work goes in until a builder takes it up.
fn halt(app: &mut App, site: Entity) {
    app.world_mut()
        .get_mut::<UnderConstructionComponent>(site)
        .expect("a site carries its construction state")
        .work = SiteWork::Halted;
}

/// The work put into the site `site`, or `None` once it is built.
fn progress_of(app: &App, site: Entity) -> Option<u32> {
    app.world()
        .get::<UnderConstructionComponent>(site)
        .map(|site| site.progress)
}
