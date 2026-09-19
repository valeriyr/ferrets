//! Timed life: an instance carrying the `lifetime` stat stands for exactly that
//! many ticks, and a buff moving the stat moves the instances already standing.

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    player_buffs::PlayerBuffDef,
    registry::ContentRegistry,
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent},
    player_buffs::PlayerBuffs,
    session::{GameSession, player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType},
    statistics::Statistics,
};

mod utils;

//
// ─── Expiry ─────────────────────────────────────────────────────────────────
//

#[test]
fn timed_life_ends_on_tick_its_stat_names() {
    let mut app = app();
    let (_, summon) = utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);

    utils::run_ticks(&mut app, 9);
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(summon)
            .is_some(),
        "a life of ten ticks is still standing on the ninth"
    );

    utils::run_ticks(&mut app, 1);
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(summon)
            .is_none(),
        "the tenth tick is the one its time runs out on"
    );
}

#[test]
fn expiry_announces_its_own_cause() {
    let mut app = app();
    utils::record_announcements(&mut app);
    utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);

    utils::run_ticks(&mut app, 10);

    assert!(
        app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(
                event,
                SimulationEvent::EntityDied {
                    cause: DeathCause::Expired,
                    ..
                }
            )),
        "a life that ran out dies of expiry, not of damage or decay"
    );
}

#[test]
fn expiry_leaves_remains_type_declares() {
    let mut app = app();
    utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);

    // Ten ticks to expire, then the two-tick dying phase the type declares.
    utils::run_ticks(&mut app, 13);

    assert_eq!(
        app.world()
            .resource::<EntityIndex>()
            .remains_entries()
            .len(),
        1,
        "the type leaves a body for a life that ran out, so expiry leaves one"
    );
}

//
// ─── Buffs move a standing life ─────────────────────────────────────────────
//

#[test]
fn buff_lengthening_stat_keeps_standing_summon_up() {
    let mut app = app();
    let (_, summon) = utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);
    let longevity = app
        .world()
        .resource::<ContentRegistry>()
        .player_buff("longevity")
        .expect("the buff is registered");

    utils::run_ticks(&mut app, 5);
    app.world_mut().resource_mut::<PlayerBuffs>().apply(
        PlayerId::from(0u8),
        longevity,
        StackRule::Ignore,
        None,
    );

    // 10 + 5 = 15: the buff lengthens the stat the age is compared against, so
    // the summon standing at five stands five ticks longer than it would have.
    utils::run_ticks(&mut app, 9);
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(summon)
            .is_some(),
        "the lengthened life carries the summon past its original ten"
    );
    utils::run_ticks(&mut app, 1);
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(summon)
            .is_none(),
        "and ends it on the fifteenth"
    );
}

#[test]
fn buff_shortening_life_past_nothing_ends_it_at_once() {
    let mut app = app();
    let withering = app
        .world()
        .resource::<ContentRegistry>()
        .player_buff("withering")
        .expect("the buff is registered");
    app.world_mut().resource_mut::<PlayerBuffs>().apply(
        PlayerId::from(0u8),
        withering,
        StackRule::Ignore,
        None,
    );
    let (_, summon) = utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);

    // 10 - 20 folds below nothing, and the age is compared against what is
    // left: a life a buff has taken past its end is over on the first tick
    // rather than wrapping round into a long one.
    utils::run_ticks(&mut app, 1);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .alive(summon)
            .is_none(),
        "a life shortened past nothing ends at once rather than standing for good"
    );
}

//
// ─── Statistics ─────────────────────────────────────────────────────────────
//

#[test]
fn expiry_is_no_loss() {
    let mut app = app();
    let (_, wisp) = utils::create_owned(&mut app, "wisp_of_ten", 5, 5, 0);

    utils::run_ticks(&mut app, 12);

    assert!(
        app.world().resource::<EntityIndex>().alive(wisp).is_none(),
        "the summon's time ran out, so the tally below is about what that counts as"
    );
    let summon = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("wisp_of_ten")
        .expect("the type is registered");
    assert_eq!(
        app.world().resource::<Statistics>().player(0).lost(summon),
        0,
        "a summon whose time ran out was never a unit to lose"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// App with one human player and a `wisp_of_ten`: a summon standing ten ticks,
/// leaving a `bones` body behind, and a `longevity` player buff lengthening
/// every timed life by five.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("bones")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(400)),
        );
        registry.register(
            EntityTypeDef::new("wisp_of_ten")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(10))
                .with_dying(2, utils::leaves("bones")),
        );
        registry.register_player_buff(
            "longevity",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: EntityStatId::LIFETIME,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(5),
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "withering",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: EntityStatId::LIFETIME,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(-20),
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
