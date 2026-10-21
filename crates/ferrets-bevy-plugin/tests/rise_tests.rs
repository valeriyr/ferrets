//! The rise: pools following its work and settling once risen, the rally a rising mover holds, and contact.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    build::BuilderAttendance,
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
    rise::{RisePool, RisePoolInitial},
    stack_rule::StackRule,
    work::{CrewLimit, WorkPresence},
};
use ferrets_geometry::{cell_size::CellSize, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    beside::Lean,
    command::PlayerCommand,
    components::{
        pools,
        rally::{RallyPointComponent, RallyTarget},
        rise::{self, RiseAttendance, RisingComponent},
    },
    entity_def,
    entity_index::EntityIndex,
    movement_model::MovementModel,
    spawn,
};

//
// ─── Pools along the rise ─────────────────────────────────────────────────────
//

#[test]
fn site_never_bears_risen_passive_tick_it_is_founded() {
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
    // `opened`, which holds only once risen.
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
fn rising_entity_gains_health_with_work() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    // A quarter of 400 at founding.
    assert_eq!(utils::health_as_u32(&app, rampart), 100);

    // (400 − 100) / 10 = 30 a tick of work, moved as it is put in:
    // 100 + 5 × 30 = 250.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);

    // The tenth tick completes it, full.
    utils::run_ticks(&mut app, 5);
    assert!(app.world().get::<RisingComponent>(rampart).is_none());
    assert_eq!(utils::health_as_u32(&app, rampart), 400);
}

#[test]
fn damage_taken_while_rising_stays_as_deficit() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 5);
    // 250, wounded by 100 to 150; the line goes on to 400: 400 − 100 = 300.
    utils::wound(&mut app, rampart, "100");
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 300);
}

#[test]
fn maximum_raised_mid_rise_still_finishes_full() {
    // Whatever shift the raise names, the line moves the rising entity, not
    // the shift. The clamp arm reads the same with the shift applied, since a
    // clamp leaves a value under a raised maximum where it is.
    for (name, pool_shift) in [
        ("fortified_by_clamp", PoolShift::Clamp),
        ("fortified_by_difference", PoolShift::Difference),
        ("fortified_by_share", PoolShift::Share),
    ] {
        let mut app = utils::orders_app();
        let fortified = register_health_shift(&mut app, name, "100", pool_shift);
        let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
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
fn risen_entity_gives_its_pool_back_to_fold() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Difference);
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::health_as_u32(&app, rampart), 400);

    // Risen, its health follows its maximum by the raise's own shift again:
    // 400 + 100 = 500.
    utils::apply_buff(app.world_mut(), rampart, fortified);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, rampart), 500);
}

#[test]
fn halted_rise_does_not_advance() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    // Five ticks of work: 100 + 5 × 30 = 250.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);

    // Halted, no work goes in and the line stands.
    halt(&mut app, rampart);
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 250);
}

#[test]
fn maximum_lowered_mid_rise_still_finishes_full() {
    // Whatever shift the cut names, the line moves the rising entity, not
    // the shift.
    for (name, pool_shift) in [
        ("weakened_by_clamp", PoolShift::Clamp),
        ("weakened_by_difference", PoolShift::Difference),
        ("weakened_by_share", PoolShift::Share),
    ] {
        let mut app = utils::orders_app();
        let weakened = register_health_shift(&mut app, name, "-200", pool_shift);
        let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
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
fn falling_line_leaves_live_rising_entity_its_last_sliver() {
    let mut app = utils::orders_app();
    let weakened = register_health_shift(&mut app, "weakened", "-200", PoolShift::Clamp);
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    utils::run_ticks(&mut app, 5);
    halt(&mut app, rampart);
    // 250 wounded by 200 to 50; under the lowered maximum the line falls from
    // 250 to 50 + 150 × 5 / 10 = 125, 75 more than the 50 left, which leaves
    // the live rising entity the smallest value.
    utils::wound(&mut app, rampart, "200");
    utils::apply_buff(app.world_mut(), rampart, weakened);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, rampart), FixedU64::DELTA);
    assert_eq!(utils::count_of_type(app.world_mut(), "rampart"), 1);
}

#[test]
fn pool_gained_on_risen_comes_at_its_initial_once_risen() {
    let mut app = utils::orders_app();
    let (tower, _) = utils::create_unattended_rising(&mut app, "flare_tower", 8, 8, 0);
    assert_eq!(
        entity_def::pool_value(app.world(), tower, PoolId::ENERGY),
        None
    );
    // The fourth tick completes it, gaining its energy at a quarter of 60,
    // 15, which the next fold settles where it stands.
    utils::run_ticks(&mut app, 4);
    assert!(app.world().get::<RisingComponent>(tower).is_none());
    assert_eq!(utils::energy_as_u32(&app, tower), 15);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, tower), 15);
}

#[test]
fn rising_pool_keeps_what_it_regenerates_besides() {
    let mut app = utils::orders_app();
    let (cistern, _) = utils::create_unattended_rising(&mut app, "cistern", 8, 8, 0);
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
            utils::rise_progress(&app, rampart),
            utils::health_as_u32(&app, rampart),
        ));
        utils::run_ticks(&mut app, 1);
    }
    // Founded at a quarter of 400, then 30 a tick of work as it is put in:
    // 100 + 30 × progress, full once risen.
    let mut expected: Vec<(Option<u32>, u32)> = (0..10)
        .map(|progress| (Some(progress), 100 + 30 * progress))
        .collect();
    expected.extend([(None, 400), (None, 400)]);
    assert_eq!(seen, expected);
}

#[test]
fn rising_entity_starts_under_maximum_its_passives_leave() {
    let mut app = utils::orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        // While it rises, a braced rampart's maximum is 200 higher.
        let braced = registry.register_entity_buff(
            "braced",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, "200")],
                    pool_shift: PoolShift::Clamp,
                })],
                lasting: Lasting::While(Requirement::Unless(Box::new(Requirement::Risen))),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("braced_rampart")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(400))
                .with_dying(2, [])
                .with_passives([braced])
                .with_rise(
                    10,
                    [(
                        PoolId::HEALTH,
                        RisePool::FollowsWork(RisePoolInitial::Share(utils::fixed("0.25"))),
                    )],
                ),
        );
    }
    let (rampart, _) = utils::create_unattended_rising(&mut app, "braced_rampart", 8, 8, 0);
    // A quarter of 400 + 200, before any tick.
    assert_eq!(utils::health_as_u32(&app, rampart), 150);

    // Five ticks of work: 150 + 450 × 5 / 10.
    utils::run_ticks(&mut app, 5);
    assert_eq!(utils::health_as_u32(&app, rampart), 375);
}

#[test]
fn rising_entity_holds_pool_from_its_own_initial() {
    let mut app = utils::orders_app();
    app.world_mut().resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("lamp_post")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(100))
            .with_pool(Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(60),
                FixedU64::ZERO,
                FixedU64::ZERO,
                PoolInitial::Share(utils::fixed("0.25")),
            ))
            .with_dying(2, [])
            .with_rise(
                4,
                utils::rise_initial_pools(&[PoolId::HEALTH, PoolId::ENERGY]),
            ),
    );
    let (post, _) = utils::create_crewed_rising(&mut app, "lamp_post", 8, 8, 0);
    // A quarter of 60.
    assert_eq!(utils::energy_as_u32(&app, post), 15);
}

#[test]
fn risen_pools_settle_under_risen_passives_at_next_fold() {
    let mut app = utils::orders_app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        // Once risen, a keep's health and energy maxima are higher.
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
                lasting: Lasting::While(Requirement::Risen),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("keep_wall")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::ONE,
                    Solidity::Solid,
                    utils::GROUND,
                )
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
                .with_rise(
                    10,
                    [
                        (
                            PoolId::HEALTH,
                            RisePool::FollowsWork(RisePoolInitial::Share(utils::fixed("0.25"))),
                        ),
                        (PoolId::ENERGY, RisePool::GainedOnRisen),
                    ],
                ),
        );
    }
    let (keep, _) = utils::create_unattended_rising(&mut app, "keep_wall", 8, 8, 0);
    utils::run_ticks(&mut app, 10);
    assert!(app.world().get::<RisingComponent>(keep).is_none());
    // Risen: its line reached the rise's 400, and its energy stands at a
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
fn settling_keeps_what_pool_gained_on_risen_lost_since_completion() {
    let mut app = utils::orders_app();
    let (tower, _) = utils::create_unattended_rising(&mut app, "flare_tower", 8, 8, 0);
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
            utils::rise_progress(&app, rampart),
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
fn halted_rise_lines_up_under_raised_maximum() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Difference);
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
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
#[should_panic(expected = "a rise's work stops at its rise time: 11 > 10")]
fn rising_past_rise_time_panics() {
    let mut app = utils::orders_app();
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    app.world_mut()
        .get_mut::<RisingComponent>(rampart)
        .expect("a rising entity carries its rising state")
        .progress = 11;
    rise::rise(app.world_mut(), rampart);
}

#[test]
fn buff_on_completing_tick_moves_rising_pool_by_its_line() {
    let mut app = utils::orders_app();
    let fortified = register_health_shift(&mut app, "fortified", "100", PoolShift::Clamp);
    let (rampart, _) = utils::create_unattended_rising(&mut app, "rampart", 8, 8, 0);
    // The tenth tick completes it at 400; wounded to 350 and buffed before
    // the next fold, its pool still follows the line.
    utils::run_ticks(&mut app, 10);
    utils::wound(&mut app, rampart, "50");
    utils::apply_buff(app.world_mut(), rampart, fortified);
    utils::run_ticks(&mut app, 1);
    // Settled by the line, 400 → 500, keeping the 50 deficit: 450 of 500,
    // where the clamp alone would leave a risen entity at 350 of 500.
    assert_eq!(utils::health_as_u32(&app, rampart), 450);
}

//
// ─── Rally ────────────────────────────────────────────────────────────────────
//

#[test]
fn rising_mover_that_trains_goes_to_its_rally_and_keeps_none() {
    let mut app = utils::orders_app();
    register_rising_trainers(&mut app);
    let (ark, ark_id) = utils::create_unattended_rising(&mut app, "ark", 8, 8, 0);
    utils::set_rally(&mut app, ark_id, 20, 8);
    utils::run_ticks(&mut app, RALLY_RISE_TIME + 1);

    // The rally had the one goal: the risen ark walks there, and starts its
    // life as a trainer with no rally for what it trains.
    assert_eq!(utils::move_target(&app, ark), Some(utils::pos(20, 8)));
    let rally = app
        .world()
        .get::<RallyPointComponent>(ark)
        .expect("a trainer keeps a rally point");
    assert_eq!(rally.0, None);
}

#[test]
fn static_rising_trainer_keeps_its_rally_for_what_it_trains() {
    let mut app = utils::orders_app();
    register_rising_trainers(&mut app);
    let (camp, camp_id) = utils::create_unattended_rising(&mut app, "camp", 8, 8, 0);
    utils::set_rally(&mut app, camp_id, 20, 8);
    utils::run_ticks(&mut app, RALLY_RISE_TIME + 1);

    assert!(!entity_def::rising(app.world(), camp));
    assert!(entity_def::orders(app.world(), camp).is_empty());
    let rally = app
        .world()
        .get::<RallyPointComponent>(camp)
        .expect("a trainer keeps a rally point");
    assert_eq!(rally.0, Some(RallyTarget::Position(utils::pos(20, 8))));
}

#[test]
fn captured_rising_mover_loses_rally_old_owner_set() {
    let mut app = utils::orders_app();
    register_rising_trainers(&mut app);
    let (ark, ark_id) = utils::create_unattended_rising(&mut app, "ark", 8, 8, 0);
    utils::set_rally(&mut app, ark_id, 20, 8);

    spawn::change_owner(app.world_mut(), ark, 1, ark_id, Lean::Nothing);

    let rally = app
        .world()
        .get::<RallyPointComponent>(ark)
        .expect("a rising mover carries a rally point");
    assert_eq!(rally.0, None);
    // Risen, it goes nowhere.
    utils::run_ticks(&mut app, RALLY_RISE_TIME + 1);
    assert!(!entity_def::rising(app.world(), ark));
    assert_eq!(utils::move_target(&app, ark), None);
}

//
// ─── Contact ──────────────────────────────────────────────────────────────────
//

#[test]
fn rising_mover_stands_where_set_down_under_contact() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    register_rising_trainers(&mut app);
    // A halted ark puts in no work and stays rising; a soldier walks onto
    // its cell.
    let (ark, _) = utils::create_rising(&mut app, "ark", 5, 5, 0, RiseAttendance::Halted);
    let (_, soldier) = utils::create_owned(&mut app, "soldier", 2, 5, 0);
    utils::select(&mut app, soldier);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(5, 5),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, 120);

    assert!(entity_def::rising(app.world(), ark));
    assert_eq!(utils::position_of(app.world_mut(), ark), utils::pos(5, 5));
    // The soldier rests against it, a full body apart (radius 0.5 each), on
    // its far side: x = 6 less the contact pass's rounding.
    let soldier_entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(soldier)
        .expect("the soldier stands");
    assert_eq!(
        utils::position_of(app.world_mut(), soldier_entity),
        utils::position_bits(0x5_ffff_ffff, 0x5_0000_0000)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

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

/// Halts the rising `entity`: no work goes in until a worker takes it up.
fn halt(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<RisingComponent>(entity)
        .expect("a rising entity carries its rising state")
        .attendance = RiseAttendance::Halted;
}

/// Ticks the `ark` and the `camp` rise over.
const RALLY_RISE_TIME: u32 = 4;

/// Registers an `ark` that moves and trains soldiers and a `camp` that stands
/// and trains them, both rising over `RALLY_RISE_TIME` with health at its
/// initial.
fn register_rising_trainers(app: &mut App) {
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    registry.register(
        utils::walker("ark", utils::GROUND_DOMAIN)
            .with_pool(Pool::health(50))
            .with_dying(1, [])
            .with_trainer(["soldier"])
            .with_rise(
                RALLY_RISE_TIME,
                utils::rise_initial_pools(&[PoolId::HEALTH]),
            ),
    );
    registry.register(
        EntityTypeDef::new("camp")
            .with_location(
                utils::GROUND_DOMAIN,
                CellSize::ONE,
                Solidity::Solid,
                utils::GROUND,
            )
            .with_pool(Pool::health(50))
            .with_dying(1, [])
            .with_trainer(["soldier"])
            .with_rise(
                RALLY_RISE_TIME,
                utils::rise_initial_pools(&[PoolId::HEALTH]),
            ),
    );
}
