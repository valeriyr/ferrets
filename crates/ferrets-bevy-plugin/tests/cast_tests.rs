//! Casting at a body: the Remains aim, the Summon effect, and the Cast order
//! that walks a caster into its reach before either happens.

use bevy::prelude::*;
use ferrets_content::{
    attack::Slain,
    cost::Cost,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    kinds::Kinds,
    location::Solidity,
    player_buffs::PlayerBuffDef,
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{Casting, EntityCastEffect, EntityCastTarget, Reach, SkillCaster, SkillDef},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    buffs_store::Term,
    command::{PlayerCommand, SkillCasterRef, SkillTarget},
    components::entity_info::EntityInfoComponent,
    entity_def,
    entity_index::EntityIndex,
    events::{SimulationEvent, SpawnCause},
    game_loop::damage,
    map::Map,
    order::Order,
    player_buffs::PlayerBuffs,
    session::{GameSession, player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    statistics::Statistics,
};

mod utils;

//
// ─── Raising from a body ────────────────────────────────────────────────────
//

#[test]
fn raise_spends_body_and_sets_down_what_it_makes() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_none(),
        "the body the cast was aimed at is spent"
    );
    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "exactly the two the skill names stand up"
    );
    assert!(
        owners(&app, "skeleton").all(|owner| owner == Some(0)),
        "what a caster raises is the caster's own"
    );
}

#[test]
fn raise_announces_body_it_spent() {
    let mut app = app();
    utils::record_announcements(&mut app);
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    let announced = &app.world().resource::<utils::Announced>().0;
    assert!(
        announced.iter().any(|event| matches!(
            event,
            SimulationEvent::RemainsSpent { remains, position, .. }
                if *remains == body && *position == utils::pos(6, 5)
        )),
        "spending a body is announced, so a game can play it"
    );
    assert!(
        announced.iter().any(|event| matches!(
            event,
            SimulationEvent::EntitySpawned {
                cause: SpawnCause::Raised { from, .. },
                ..
            } if *from == body
        )),
        "what comes up remembers the body it came out of"
    );
}

#[test]
fn raising_is_not_production() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "the raise happened, so the tally below is about what it counts as"
    );
    let skeleton = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("skeleton")
        .expect("the type is registered");
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .produced(skeleton),
        0,
        "a spell that lasts a while is not a unit anyone produced"
    );
}

#[test]
fn raise_aimed_at_living_is_refused() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let (_, standing) = utils::create_owned(&mut app, "soldier", 6, 5, 1);

    raise(&mut app, necromancer, standing);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a raise names a body, and what still stands is not one"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "a refused cast pays nothing"
    );
}

#[test]
fn summon_with_nowhere_to_stand_pays_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // The colossus is wider than the map, so no cell can hold one: the cast is
    // refused before anything is spent.
    cast(&mut app, necromancer, body, "raise_colossus");
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "colossus"),
        0,
        "a summon with nowhere to stand raises nothing"
    );
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "nothing was paid for the cast that did not happen"
    );
}

#[test]
fn summon_over_supply_ceiling_pays_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Two wardens cost two supply each and the player provides none, so half a
    // summon is not offered: the cast does nothing at all.
    cast(&mut app, necromancer, body, "raise_wardens");
    utils::run_ticks(&mut app, 5);

    assert_eq!(count_of(&app, "warden"), 0, "half a summon is not offered");
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and nothing was paid for it"
    );
}

//
// ─── The walk into reach ────────────────────────────────────────────────────
//

#[test]
fn caster_walks_into_its_reach_before_casting() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    raise(&mut app, necromancer, body);
    // Twelve cells away with a reach of four: long past the tick an unranged
    // caster would have cast on, and still nothing has been raised.
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a caster out of reach walks rather than casting across the map"
    );

    utils::run_ticks(&mut app, 120);
    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "having closed to its reach, it casts"
    );
}

#[test]
fn body_gone_before_caster_arrives_ends_order() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);
    // Someone else's necromancer got there first.
    let other = caster(&mut app, 13, 5);
    raise(&mut app, other, body);
    utils::run_ticks(&mut app, 120);

    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(necromancer)
        .expect("the caster is still standing");
    assert!(
        utils::order_queue_is_empty(app.world_mut(), entity),
        "an aim that is gone finishes the order rather than holding the caster"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and the caster paid nothing for the body it never reached"
    );
}

#[test]
fn caster_without_reach_casts_where_it_stands() {
    let mut app = app();
    // The acolyte's raise declares no reach, so it lands at once however far
    // off the body is.
    let acolyte = summoner(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    cast(&mut app, acolyte, body, "raise_where_it_stands");
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "an unranged cast is instant, as every cast was before reaches existed"
    );
}

#[test]
fn body_in_fog_cannot_be_raised() {
    let mut app = app();
    // The acolyte sees twenty cells and casts where it stands, so what stops
    // this raise is the fog over the body and nothing else.
    let acolyte = summoner(&mut app, 2, 2);
    let body = fallen(&mut app, 29, 29);

    cast(&mut app, acolyte, body, "raise_where_it_stands");
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a cast named at what the fog hides is a cast the player could not have known to make"
    );
    assert_eq!(
        energy(&app, acolyte),
        FixedU64::from_num(100),
        "and it paid nothing for it"
    );
}

#[test]
fn summon_with_room_for_only_one_raises_none() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Every cell on the map held but the one beside the body, so the search
    // around it finds exactly one place to stand where the raise needs two.
    // A body lies on held ground, so this leaves the aim itself untouched.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    free_cell(&mut app, CellPos::new(7, 5));

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(count_of(&app, "skeleton"), 0, "half a raise is not offered");
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and nothing was paid for it"
    );
}

//
// ─── Working at a cast ──────────────────────────────────────────────────────
//

#[test]
fn worked_cast_lands_on_its_point() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    cast(&mut app, necromancer, body, "raise_slowly");
    // The command waits out the input delay (utils::APPLY = 3), and the tick
    // it applies on is the first tick of work: the skeletons rise on the fifth
    // of those, the seventh from here — 3 + 5 - 1.
    utils::run_ticks(&mut app, 6);

    assert_eq!(count_of(&app, "skeleton"), 0, "the work is not done yet");
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and nothing is spent while it is under way"
    );

    utils::run_ticks(&mut app, 1);

    assert_eq!(count_of(&app, "skeleton"), 2, "the fifth tick raises them");
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(60),
        "and pays for them then: 100 - 40"
    );
}

#[test]
fn worked_cast_walks_in_before_its_work_starts() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    // Twelve cells out with a reach of four: the work cannot start until the
    // walk is over, so the cast lands far later than its five-tick point.
    cast(&mut app, necromancer, body, "raise_slowly");
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a caster still walking is a caster not yet working"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and it pays for the cast at its point, not for setting off"
    );

    utils::run_ticks(&mut app, 120);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "having closed, it works and raises"
    );
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(60),
        "paying then: 100 - 40"
    );
}

#[test]
fn caster_stands_through_what_is_left_of_its_cast() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    cast(&mut app, necromancer, body, "raise_slowly");
    // Landed on the seventh tick, with a period of eight: three more to stand
    // through before the order is done with it.
    utils::run_ticks(&mut app, 8);

    assert_eq!(count_of(&app, "skeleton"), 2, "the cast has landed");
    assert!(
        !orders_of(&app, necromancer).is_empty(),
        "and the caster is still at it"
    );

    utils::run_ticks(&mut app, 2);

    assert!(
        orders_of(&app, necromancer).is_empty(),
        "the cast lets go once its period is out"
    );
}

#[test]
fn point_pulled_under_work_already_done_lands_at_once() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    cast(&mut app, necromancer, body, "raise_by_stat");
    // Four ticks of work against a point of ten, then a buff pulls the point
    // to two — under the work already done. A cast has no next cycle to fire
    // on, so it lands the very next tick rather than being skipped.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(count_of(&app, "skeleton"), 0, "the work is not done yet");

    apply_buff(&mut app, "quickened_rites");
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "a point that moved under the phase already worked lands on the next tick"
    );
}

#[test]
fn cast_landing_past_its_period_frees_caster_at_once() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // A buff draws the point out to twenty, past the twelve-tick period the
    // skill is freed at — content may not author that, but a stat may reach
    // it. The cast lands on the twentieth tick of work with the period long
    // gone, so the caster is free the same tick it casts.
    apply_buff(&mut app, "drawn_out_rites");
    cast(&mut app, necromancer, body, "raise_by_stat");
    utils::run_ticks(&mut app, utils::APPLY + 19);

    assert_eq!(count_of(&app, "skeleton"), 2, "the cast landed");
    assert!(
        orders_of(&app, necromancer).is_empty(),
        "and nothing holds the caster past a period its point outran"
    );
}

#[test]
fn cast_lands_once_though_its_cooldown_comes_round_first() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Five ticks to the point, eight to the end, and two of cooldown: the
    // skill is ready again while the caster is still standing through its own
    // cast, so only the cast's own stage stops it raising twice.
    cast(&mut app, necromancer, body, "raise_slowly");
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "a cast lands once, whatever the phase does after it"
    );
}

#[test]
fn cast_that_kills_its_caster_ends_as_death() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);

    // The queue is out of the world while an order runs, so a caster the cast
    // killed would keep a dying phase nothing counts down unless the order
    // itself says it died.
    utils::use_skill(
        &mut app,
        "self_immolate",
        SkillCasterRef::Entity(necromancer),
        None,
    );
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .any(necromancer)
            .is_none(),
        "a caster its own cast killed leaves the world rather than lingering dying"
    );
}

#[test]
fn skill_pressed_on_cooldown_leaves_caster_at_its_work() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    // Both bodies first: laying one costs ticks, and what this test is about
    // is how few of them pass between the raise and the press that follows it.
    let body = fallen(&mut app, 6, 5);
    let second = fallen(&mut app, 7, 5);

    // One raise, then a walk, then the same skill pressed while it is still
    // cooling — three ticks of a ten-tick cooldown gone. The walk must
    // survive: a cast that will not happen is no reason to drop what the
    // caster was doing.
    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    utils::select(&mut app, necromancer);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 12),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    raise(&mut app, necromancer, second);
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(
        orders_of(&app, necromancer)
            .iter()
            .any(|order| matches!(order, Order::Move { .. })),
        "the walk the caster was on outlives a cast that never starts"
    );
}

#[test]
fn cast_cut_short_before_its_point_costs_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    cast(&mut app, necromancer, body, "raise_slowly");
    // Two ticks in: the work has started and has three more to run.
    utils::run_ticks(&mut app, 4);
    // Sent somewhere else mid-cast: the order goes, and the work with it.
    utils::select(&mut app, necromancer);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 12),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert_eq!(count_of(&app, "skeleton"), 0, "nothing was raised");
    assert_eq!(
        energy(&app, necromancer),
        FixedU64::from_num(100),
        "and the energy was never spent"
    );
    assert_eq!(
        app.world()
            .resource::<EntityIndex>()
            .remains_entries()
            .len(),
        1,
        "the body it was working on still lies there"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// App with two players and the raising roster: `raise_dead`, reaching four
/// cells, on a `necromancer` that can walk; `raise_where_it_stands`, reaching
/// nowhere, on a rooted `acolyte`; both cost 40 energy and make two
/// `skeleton`s from a body. Plus a `soldier` that leaves a `corpse` and two
/// summons that can never be set down — a `colossus` wider than the map and a
/// `warden` costing supply nobody provides.
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("corpse")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
        );
        registry.register(
            EntityTypeDef::new("skeleton")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20),
        );
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(30)
                .with_dying(2, utils::leaves("corpse")),
        );
        registry.register(
            EntityTypeDef::new("colossus")
                .with_location(utils::GROUND, CellSize::new(40, 40), Solidity::Solid)
                .with_health(20),
        );
        registry.register(
            EntityTypeDef::new("warden")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::from_num(2)),
        );
        let raises = |registry: &mut ContentRegistry, name: &str, summoned: &str, reach| {
            let entity_type = registry
                .type_id(summoned)
                .expect("the summoned type is registered");
            registry.register_skill(
                name,
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                        reach,
                        casting: Casting::Instant,
                        effect: EntityCastEffect::Summon {
                            entity_type,
                            count: 2,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        let raise_dead = raises(
            &mut registry,
            "raise_dead",
            "skeleton",
            Reach::Within(Quantity::Constant(4)),
        );
        let raise_colossus = raises(
            &mut registry,
            "raise_colossus",
            "colossus",
            Reach::Within(Quantity::Constant(4)),
        );
        let raise_wardens = raises(
            &mut registry,
            "raise_wardens",
            "warden",
            Reach::Within(Quantity::Constant(4)),
        );
        // A cast that brings its own caster down.
        let self_immolate = registry.register_skill(
            "self_immolate",
            SkillDef {
                cooldown: 10,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Caster,
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Damage(FixedU64::from_num(1000)),
                },
                requires: Vec::new(),
            },
        );
        // A raise the caster works at: the skeletons rise on the fifth tick and
        // it stands through three more before it takes another order. Its
        // cooldown is shorter than that hold, so nothing but the cast's own
        // stage stops it landing again while the caster stands.
        let raise_slowly = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_slowly",
                SkillDef {
                    cooldown: 2,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                        reach: Reach::Within(Quantity::Constant(4)),
                        casting: Casting::Delayed {
                            point: Quantity::Constant(5),
                            period: Quantity::Constant(8),
                        },
                        effect: EntityCastEffect::Summon {
                            entity_type,
                            count: 2,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        // A raise worked over a stat: what the caster reads for its point is
        // `ritual_time`, which a buff can move either way under it mid-cast.
        // Content may not author a point outside its period, so the two buffs
        // are what put it there.
        let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ONE);
        let raise_by_stat = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_by_stat",
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        target: EntityCastTarget::Fallen { kinds: Kinds::Any },
                        reach: Reach::Within(Quantity::Constant(4)),
                        casting: Casting::Delayed {
                            point: Quantity::Stat(ritual_time),
                            period: Quantity::Constant(12),
                        },
                        effect: EntityCastEffect::Summon {
                            entity_type,
                            count: 2,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        registry.register_player_buff(
            "drawn_out_rites",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: ritual_time,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(10),
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "quickened_rites",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: ritual_time,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(-8),
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        // The same raise, cast from wherever the caster stands.
        let raise_where_it_stands = raises(
            &mut registry,
            "raise_where_it_stands",
            "skeleton",
            Reach::Wherever,
        );
        registry.register(
            utils::walker("necromancer", utils::GROUND)
                .with_health(40)
                .with_sight_range(20)
                .with_energy(100, FixedU64::ZERO)
                .with_stat(ritual_time, FixedU64::from_num(10))
                .with_skills([
                    raise_dead,
                    raise_colossus,
                    raise_wardens,
                    raise_slowly,
                    raise_by_stat,
                    self_immolate,
                ]),
        );
        registry.register(
            EntityTypeDef::new("acolyte")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(40)
                .with_sight_range(20)
                .with_energy(100, FixedU64::ZERO)
                .with_skills([raise_where_it_stands]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Puts a player buff on everything player 0 owns, standing and to come.
fn apply_buff(app: &mut App, name: &str) {
    let buff = app
        .world()
        .resource::<ContentRegistry>()
        .player_buff(name)
        .expect("the buff is registered");
    app.world_mut().resource_mut::<PlayerBuffs>().apply(
        PlayerId::from(0u8),
        buff,
        StackRule::Ignore,
        Term::Forever,
    );
}

/// Frees one statically occupied cell, so a test can leave exactly as much
/// room as it means to.
fn free_cell(app: &mut App, cell: CellPos) {
    let mut map = app.world_mut().resource_mut::<Map>();
    map.set_static_occupied(utils::GROUND, cell, false);
}

/// A necromancer of player 0 at `(x, y)`.
fn caster(app: &mut App, x: u32, y: u32) -> SimulationId {
    utils::create_owned(app, "necromancer", x, y, 0).1
}

/// An acolyte of player 0 at `(x, y)` — a caster with no reach of its own.
fn summoner(app: &mut App, x: u32, y: u32) -> SimulationId {
    utils::create_owned(app, "acolyte", x, y, 0).1
}

/// Kills a soldier of player 1 at `(x, y)` and returns the body it leaves.
fn fallen(app: &mut App, x: u32, y: u32) -> SimulationId {
    let (soldier, _) = utils::create_owned(app, "soldier", x, y, 1);
    damage::apply(
        app.world_mut(),
        SimulationId(u32::MAX),
        soldier,
        FixedU64::from_num(1000),
        Slain::Remains,
    );
    utils::run_ticks(app, 4);
    app.world()
        .resource::<EntityIndex>()
        .remains_entries()
        .last()
        .expect("the soldier left a body")
        .0
}

/// Commands `caster` to raise `body` with `raise_dead`.
fn raise(app: &mut App, caster: SimulationId, body: SimulationId) {
    cast(app, caster, body, "raise_dead");
}

/// Commands `caster` to cast `skill` on `body`.
fn cast(app: &mut App, caster: SimulationId, body: SimulationId, skill: &str) {
    utils::use_skill(
        app,
        skill,
        SkillCasterRef::Entity(caster),
        Some(SkillTarget::Entity(body)),
    );
}

/// The orders `unit` holds.
fn orders_of(app: &App, unit: SimulationId) -> Vec<Order> {
    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(unit)
        .expect("the caster is standing");
    entity_def::orders(app.world(), entity).to_vec()
}

/// How many entities of `type_name` stand.
fn count_of(app: &App, type_name: &str) -> usize {
    owners_of(app, type_name).count()
}

/// The owner of every standing entity of `type_name`.
fn owners(app: &App, type_name: &str) -> impl Iterator<Item = Option<u8>> {
    owners_of(app, type_name)
}

fn owners_of(app: &App, type_name: &str) -> impl Iterator<Item = Option<u8>> {
    let world = app.world();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter(move |&(_, entity)| {
            world
                .entity(entity)
                .get::<EntityInfoComponent>()
                .is_some_and(|info| info.type_name() == type_name)
        })
        .map(move |(_, entity)| entity_def::owner(world, entity))
}

/// The caster's current energy.
fn energy(app: &App, caster: SimulationId) -> FixedU64 {
    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(caster)
        .expect("the caster is standing");
    utils::energy(app, entity)
}
