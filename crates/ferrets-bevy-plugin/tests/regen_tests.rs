//! Health regeneration: pools refill toward their ceiling, settle back under one
//! that has dropped, and stay put for entities that must not mend on their own.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, location::Solidity,
    registry::ContentRegistry, stats::ModifierOp,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    components::build::{SiteWork, UnderConstructionComponent},
    game_loop,
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
        utils::current_health(&app, troll),
        FixedU64::from_num(32),
        "four ticks at 0.5 per tick restore two points"
    );

    utils::run_ticks(&mut app, 100);
    assert_eq!(
        utils::current_health(&app, troll),
        FixedU64::from_num(40),
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
        utils::current_health(&app, dummy),
        FixedU64::from_num(15),
        "a type with no health_regen never recovers a point"
    );
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
    game_loop::stats::apply_entity_buff(app.world_mut(), troll, frailty);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        utils::current_health(&app, troll),
        FixedU64::from_num(15),
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
    game_loop::stats::apply_entity_buff(app.world_mut(), troll, withering);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::current_health(&app, troll),
        FixedU64::ONE,
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
        utils::current_health(&app, troll),
        FixedU64::from_num(30),
        "an entity seeing out its dying phase does not heal back out of it"
    );
}

#[test]
fn entity_under_construction_does_not_regenerate() {
    let mut app = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    utils::wound(&mut app, troll, "10");
    app.world_mut()
        .entity_mut(troll)
        .insert(UnderConstructionComponent {
            progress: 0,
            work: SiteWork::Crew {
                builders: Default::default(),
            },
        });

    utils::run_ticks(&mut app, 10);
    assert_eq!(
        utils::current_health(&app, troll),
        FixedU64::from_num(30),
        "an unfinished entity has to be completed rather than mend itself"
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
    assert_eq!(utils::energy(&app, lamp), FixedU64::from_num(2));
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy(&app, lamp), FixedU64::ZERO);
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy(&app, lamp), FixedU64::ZERO);
}

#[test]
fn energy_regenerates_before_it_drains() {
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
    game_loop::stats::apply_entity_buff(app.world_mut(), lamp, trickle);

    // Each tick heals 1 (capped at 20) then drains 3: 20 → 17 → 15 → 13.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::energy(&app, lamp), FixedU64::from_num(13));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player, a `troll` that regenerates half a point per tick toward its
/// 40, a `dummy` of the same size that regenerates nothing, and a `lamp` whose
/// 20 energy drains three a tick.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("troll")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(40)
                .with_dying(3, [])
                .with_stat(EntityStatId::HEALTH_REGEN, FixedU64::from_num(0.5)),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20),
        );
        registry.register(
            EntityTypeDef::new("lamp")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_energy(20, FixedU64::ZERO)
                .with_stat(EntityStatId::ENERGY_DRAIN, FixedU64::from_num(3)),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
