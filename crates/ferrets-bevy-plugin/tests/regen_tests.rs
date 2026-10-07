//! Health regeneration: pools refill toward their ceiling, settle back under one
//! that has dropped, and stay put for the dying and where content holds them.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_buffs::{EntityBuffDef, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    registry::ContentRegistry,
    requirement::Requirement,
    stack_rule::StackRule,
    stats::ModifierOp,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

//
// ─── Regeneration toward the ceiling ────────────────────────────────────────
//

#[test]
fn health_regenerates_toward_max_and_holds_there() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    utils::wound(&mut app, troll, "10");

    // A fractional rate accumulates across ticks rather than truncating away.
    utils::run_ticks(&mut app, 4);
    assert_eq!(
        utils::health_as_u32(&app, troll),
        32,
        "four ticks at 0.5 per tick restore two points"
    );

    utils::run_ticks(&mut app, 100);
    assert_eq!(
        utils::health_as_u32(&app, troll),
        40,
        "regeneration stops at the entity's maximum health"
    );
}

#[test]
fn entity_without_regeneration_stays_wounded() {
    let mut app = app();
    let (dummy, _) = utils::create_owned(&mut app, "dummy", 8, 8, 0);
    utils::wound(&mut app, dummy, "5");

    utils::run_ticks(&mut app, 20);
    assert_eq!(
        utils::health_as_u32(&app, dummy),
        15,
        "a type with no health_regen never recovers a point"
    );
}

#[test]
fn site_regenerates_by_its_own_rate() {
    let mut app = app();
    let (troll, _) = utils::create_crewed_site(&mut app, "troll", 5, 5, 0);
    utils::wound(&mut app, troll, "10");

    // The troll's own rate holds on a site: four ticks at 0.5 restore two
    // points.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::health_as_u32(&app, troll), 32);
}

#[test]
fn site_does_not_regenerate_by_rate_held_while_built() {
    let mut app = app();
    let (site, _) = utils::create_crewed_site(&mut app, "hut", 5, 5, 0);
    let (built, _) = utils::create_owned(&mut app, "hut", 7, 5, 0);
    utils::wound(&mut app, site, "10");
    utils::wound(&mut app, built, "10");

    // `knitting` holds once built: the built hut mends 0.5 a tick, 30 → 32
    // in four ticks; the site holds at 30.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::health_as_u32(&app, site), 30);
    assert_eq!(utils::health_as_u32(&app, built), 32);
}

//
// ─── The current-under-maximum invariant ────────────────────────────────────
//

#[test]
fn health_settles_under_lowered_ceiling() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);

    let frailty = utils::register_entity_buff(
        &mut app,
        "frailty",
        EntityStatId::MAX_HEALTH,
        ModifierOp::FlatAdd,
        "-25",
        None,
    );
    utils::apply_buff(app.world_mut(), troll, frailty);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        utils::health_as_u32(&app, troll),
        15,
        "a full pool follows its ceiling down to the reduced maximum"
    );
}

#[test]
fn lowered_ceiling_does_not_kill() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);

    // Deep enough to zero the ceiling outright, which the max_health floor forbids.
    let withering = utils::register_entity_buff(
        &mut app,
        "withering",
        EntityStatId::MAX_HEALTH,
        ModifierOp::PercentAdd,
        "-1",
        None,
    );
    utils::apply_buff(app.world_mut(), troll, withering);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::health_as_u32(&app, troll),
        1,
        "the pool bottoms out at one point instead of dying to the debuff"
    );
}

//
// ─── Entities left alone ────────────────────────────────────────────────────
//

#[test]
fn dying_entity_does_not_regenerate() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    utils::wound(&mut app, troll, "10");
    spawn::destroy_entity(app.world_mut(), troll);

    utils::run_ticks(&mut app, 2);
    assert_eq!(
        utils::health_as_u32(&app, troll),
        30,
        "an entity seeing out its dying phase does not heal back out of it"
    );
}

//
// ─── Energy flow ────────────────────────────────────────────────────────────
//

#[test]
fn energy_drains_by_its_stat_and_floors_at_empty() {
    let mut app = app();
    let (lamp, _) = utils::create_owned(&mut app, "lamp", 5, 5, 0);

    // 20 energy draining 3 a tick with no regeneration: 20 − 6 × 3 = 2 after
    // six ticks, then the seventh takes it to empty and it stays there.
    utils::run_ticks(&mut app, 6);
    assert_eq!(utils::energy_as_u32(&app, lamp), 2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, lamp), 0);
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy_as_u32(&app, lamp), 0);
}

#[test]
fn energy_flows_by_regeneration_net_of_drain() {
    let mut app = app();
    let (lamp, _) = utils::create_owned(&mut app, "lamp", 5, 5, 0);
    let trickle = utils::register_entity_buff(
        &mut app,
        "trickle",
        EntityStatId::ENERGY_REGEN,
        ModifierOp::FlatAdd,
        "1",
        None,
    );
    utils::apply_buff(app.world_mut(), lamp, trickle);

    // Each tick moves by 1 − 3 = −2: 20 → 18 → 16 → 14.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy_as_u32(&app, lamp), 14);
}

#[test]
fn balanced_health_flow_holds_full_pool() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    let wasting = utils::register_entity_buff(
        &mut app,
        "wasting",
        EntityStatId::HEALTH_DRAIN,
        ModifierOp::FlatAdd,
        "0.5",
        None,
    );
    utils::apply_buff(app.world_mut(), troll, wasting);

    // Each tick moves by 0.5 − 0.5 = 0: the full 40 stays 40.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::health(&app, troll), utils::fixed("40"));
}

#[test]
fn balanced_flow_holds_full_pool() {
    let mut app = app();
    let (lamp, _) = utils::create_owned(&mut app, "lamp", 5, 5, 0);
    let spring = utils::register_entity_buff(
        &mut app,
        "spring",
        EntityStatId::ENERGY_REGEN,
        ModifierOp::FlatAdd,
        "3",
        None,
    );
    utils::apply_buff(app.world_mut(), lamp, spring);

    // Each tick moves by 3 − 3 = 0: the full 20 stays 20.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy_as_u32(&app, lamp), 20);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player, a `troll` that regenerates half a point per tick toward its
/// 40, a `dummy` of the same size that regenerates nothing, a `lamp` whose
/// 20 energy drains three a tick, and a `hut` (40 health) whose passive
/// `knitting` regenerates half a point per tick once it is built.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("troll")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_build(4, utils::site_initial(&[PoolId::HEALTH]))
                .with_pool(Pool::builtin(
                    PoolId::HEALTH,
                    FixedU64::from_num(40),
                    utils::fixed("0.5"),
                    FixedU64::ZERO,
                    PoolInitial::Full,
                ))
                .with_dying(3, []),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20)),
        );
        let knitting = registry.register_entity_buff(
            "knitting",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::HEALTH_REGEN, "0.5"),
                ]))],
                lasting: Lasting::While(Requirement::Built),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("hut")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_build(4, utils::site_initial(&[PoolId::HEALTH]))
                .with_pool(Pool::health(40))
                .with_passives([knitting]),
        );
        registry.register(
            EntityTypeDef::new("lamp")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_pool(Pool::builtin(
                    PoolId::ENERGY,
                    FixedU64::from_num(20),
                    FixedU64::ZERO,
                    FixedU64::from_num(3),
                    PoolInitial::Full,
                )),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
