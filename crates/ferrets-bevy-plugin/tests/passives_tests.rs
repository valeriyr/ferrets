//! Buffs held on a requirement: passives applied while it is met, ended when it lapses.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    attack::Slain,
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        build,
        concealed::ConcealedComponent,
        entity_buffs::BuffsComponent,
        pools::{self, Spending},
    },
    entity_def,
    events::{DeathCause, SimulationEvent},
    game_loop,
    player_stats::PlayerStats,
    requirements,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
    statistics::Statistics,
    visibility::Sighting,
};

//
// ─── The line ───────────────────────────────────────────────────────────────
//

#[test]
fn passive_holds_under_line_and_lapses_over_it() {
    let (mut app, on_fire) = app();
    let (shed, _) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // 100 of 100: over the half line, nothing borne, nothing drained.
    assert!(!entity_def::bears(app.world(), shed, on_fire));

    // 50 of 100 is on the line, not under it: still nothing.
    utils::wound(&mut app, shed, "50");
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), shed, on_fire));
    assert_eq!(utils::health_as_u32(&app, shed), 50);

    // 49 of 100: under the line. The refit that opens the next tick fits the
    // buff, that tick's fold carries its drain, and that tick's flow takes
    // 2: 49 − 2 = 47.
    utils::wound(&mut app, shed, "1");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), shed, on_fire));
    assert_eq!(utils::health_as_u32(&app, shed), 47);

    // Lifted to 51: the next refit finds the line cleared and ends the buff,
    // and that tick drains nothing more — 51 holds.
    pools::restore(app.world_mut(), shed, PoolId::HEALTH, utils::fixed("4"));
    assert_eq!(utils::health_as_u32(&app, shed), 51);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), shed, on_fire));
    assert_eq!(utils::health_as_u32(&app, shed), 51);
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::health_as_u32(&app, shed), 51);
}

#[test]
fn burn_takes_from_tick_after_crossing() {
    let (mut app, _) = app();
    let (shed, _) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Wounded under the line mid-flight of tick t; t's flow has already run,
    // so the pool opens t+1 at 40 and closes it at 38.
    utils::wound(&mut app, shed, "60");
    assert_eq!(utils::health_as_u32(&app, shed), 40);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, shed), 38);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, shed), 36);
}

#[test]
fn burn_runs_pool_dry_as_decay_with_no_kill_credited() {
    let (mut app, _) = app();
    utils::record_announcements(&mut app);
    let (shed, shed_id) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    let (_, killer) = utils::create_owned(&mut app, "shed", 8, 8, 1);
    utils::run_ticks(&mut app, 1);

    // A rival's hit takes it under the line; the fire does the rest. 97 down
    // to 3, then 2 a tick: 1 after the first tick, empty after the second.
    game_loop::damage::apply(
        app.world_mut(),
        killer,
        shed,
        utils::fixed("97"),
        Slain::Remains,
    );
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, shed), 1);
    assert!(
        !app.world()
            .resource::<utils::Announced>()
            .0
            .iter()
            .any(|event| matches!(event, SimulationEvent::EntityDied { .. })),
        "still standing at 1"
    );
    utils::run_ticks(&mut app, 1);

    let announced = app.world().resource::<utils::Announced>();
    assert!(
        announced.0.iter().any(|event| matches!(
            event,
            SimulationEvent::EntityDied {
                entity,
                cause: DeathCause::Decayed,
                ..
            } if *entity == shed_id
        )),
        "a pool the fire empties dies of decay"
    );
    let shed_type = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("shed")
        .unwrap();
    let statistics = app.world().resource::<Statistics>();
    assert_eq!(statistics.player(0).lost(shed_type), 1);
    assert_eq!(statistics.player(1).killed(shed_type), 0);
}

#[test]
fn burn_is_no_hit() {
    let (mut app, _) = app();
    utils::record_announcements(&mut app);
    let (shed, _) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, shed, "60");
    utils::run_ticks(&mut app, 10);

    // Ten burning ticks: 40 − 20 = 20, and not one hit announced or tallied.
    assert_eq!(utils::health_as_u32(&app, shed), 20);
    let announced = app.world().resource::<utils::Announced>();
    assert!(
        !announced
            .0
            .iter()
            .any(|event| matches!(event, SimulationEvent::DamageLanded { .. })),
        "a drain lands no damage"
    );
    assert_eq!(
        app.world()
            .resource::<Statistics>()
            .player(0)
            .damage_taken(),
        FixedU64::ZERO
    );
}

#[test]
fn regeneration_nets_against_burn() {
    let (mut app, on_fire) = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // 40 of 100 under the line: each tick moves by 0.5 − 2 = −1.5.
    // 40 → 38.5 → 37.
    utils::wound(&mut app, troll, "60");
    utils::run_ticks(&mut app, 2);
    assert!(entity_def::bears(app.world(), troll, on_fire));
    assert_eq!(utils::health_as_u32(&app, troll), 37);
}

//
// ─── Who bears ──────────────────────────────────────────────────────────────
//

#[test]
fn site_burns_like_any_entity() {
    let (mut app, on_fire) = app();
    let (shed, _) = utils::create_crewed_site(&mut app, "shed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // A site bears the passive its requirement meets and flows like anything
    // else: 40 under the line, then 38 and 36.
    utils::wound(&mut app, shed, "60");
    assert_eq!(utils::health_as_u32(&app, shed), 40);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), shed, on_fire));
    assert_eq!(utils::health_as_u32(&app, shed), 38);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, shed), 36);
}

#[test]
fn passive_held_while_built_waits_for_completion() {
    let (mut app, _) = app();
    let (beacon, _) = utils::create_crewed_site(&mut app, "beacon", 5, 5, 0);
    let finished = buff_id(&app, "finished");
    assert!(!entity_def::bears(app.world(), beacon, finished));
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), beacon, finished));

    build::mark_as_built(app.world_mut(), beacon);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), beacon, finished));
}

#[test]
fn passive_held_unless_built_lasts_while_site() {
    let (mut app, _) = app();
    let (beacon, _) = utils::create_crewed_site(&mut app, "beacon", 5, 5, 0);
    let scaffold = buff_id(&app, "scaffold");
    assert!(entity_def::bears(app.world(), beacon, scaffold));
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), beacon, scaffold));

    build::mark_as_built(app.world_mut(), beacon);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), beacon, scaffold));
}

#[test]
fn site_keeps_passive_not_held_while_built() {
    let (mut app, _) = app();
    let (seedling, _) = utils::create_crewed_site(&mut app, "seedling", 5, 5, 0);
    let hardy = buff_id(&app, "hardy");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), seedling, hardy));
}

#[test]
fn dying_entity_bears_no_passive() {
    let (mut app, on_fire) = app();
    let (shed, _) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    spawn::despawn_entity(app.world_mut(), shed, DeathCause::Canceled);
    utils::wound(&mut app, shed, "60");
    utils::run_ticks(&mut app, 1);

    assert!(!entity_def::bears(app.world(), shed, on_fire));
}

#[test]
fn ownerless_bearer_meets_no_player_leaf() {
    let (mut app, _) = app();
    let (lantern, _) = utils::create_owned(&mut app, "lantern", 5, 5, 0);
    let (stray, _) =
        utils::create_entity(app.world_mut(), "lantern", utils::pos(8, 8), None).unwrap();
    utils::run_ticks(&mut app, 1);

    // `lit` holds while the owner has a standing shed: the owned lantern
    // bears it once a shed stands, the stray never.
    let lit = buff_id(&app, "lit");
    assert!(!entity_def::bears(app.world(), lantern, lit));
    utils::create_owned(&mut app, "shed", 12, 12, 0);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), lantern, lit));
    assert!(!entity_def::bears(app.world(), stray, lit));
}

//
// ─── Other terms and other appliers ─────────────────────────────────────────
//

#[test]
fn cast_applied_while_buff_ends_when_its_requirement_lapses() {
    let (mut app, on_fire) = app();
    // The lantern names no passive: the fire is put on it from outside, and
    // still goes out the tick the pool clears the line.
    let (lantern, _) = utils::create_owned(&mut app, "lantern", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, lantern, "60");
    utils::apply_buff(app.world_mut(), lantern, on_fire);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), lantern, on_fire));
    assert_eq!(utils::health_as_u32(&app, lantern), 38);

    pools::restore(app.world_mut(), lantern, PoolId::HEALTH, utils::fixed("20"));
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), lantern, on_fire));
    assert_eq!(utils::health_as_u32(&app, lantern), 58);
}

#[test]
fn conceal_from_passive_holds_while_idle() {
    let (mut app, _) = app();
    utils::create_owned(&mut app, "lantern", 5, 5, 1);
    let (huntress, huntress_id) = utils::create_owned(&mut app, "huntress", 7, 7, 0);
    utils::run_ticks(&mut app, 2);

    // Standing idle in the rival's sight: glimpsed, not seen.
    assert_eq!(utils::sighting_of(&app, 1, huntress), Sighting::Glimpsed);

    utils::select(&mut app, huntress_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(20, 20),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::sighting_of(&app, 1, huntress), Sighting::Seen);
}

#[test]
fn conceal_from_passive_lands_same_tick_as_its_buff() {
    let (mut app, _) = app();
    let (huntress, _) = utils::create_owned(&mut app, "huntress", 7, 7, 0);

    // Idle from the first tick: the refit fits shadowmeld, and the
    // concealment marker that runs after it reads the buff the same tick.
    utils::run_ticks(&mut app, 1);
    let shadowmeld = buff_id(&app, "shadowmeld");
    assert!(entity_def::bears(app.world(), huntress, shadowmeld));
    assert!(app.world().get::<ConcealedComponent>(huntress).is_some());
}

#[test]
fn borne_passive_is_not_applied_again() {
    let (mut app, _) = app();
    let (kiln, _) = utils::create_owned(&mut app, "kiln", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, kiln, "60");

    // `embers` stacks to five, but a passive borne is not applied again: one
    // stack draining 1 a tick, 40 − 3 = 37 after three ticks (a stack a tick
    // would have taken 1 + 2 + 3 = 6).
    utils::run_ticks(&mut app, 3);
    let embers = buff_id(&app, "embers");
    let stacks = app
        .world()
        .get::<BuffsComponent>(kiln)
        .unwrap()
        .active()
        .find(|(id, _)| *id == embers)
        .map(|(_, stacks)| stacks);
    assert_eq!(stacks, Some(1));
    assert_eq!(utils::health_as_u32(&app, kiln), 37);
}

#[test]
fn disabled_building_still_burns() {
    let (mut app, on_fire) = app();
    let (shed, _) = utils::create_owned(&mut app, "shed", 5, 5, 0);
    let stunned = buff_id(&app, "stunned");
    utils::apply_buff(app.world_mut(), shed, stunned);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, shed, "60");

    // Switched off, it still bears the fire and drains 2 a tick: 40 → 38.
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), shed, on_fire));
    assert_eq!(utils::health_as_u32(&app, shed), 38);
}

#[test]
fn change_of_form_drops_old_passives_and_fits_new_ones() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), sapling, on_fire));

    // Still under half its health as an oak, whose type names no fire: the
    // change drops `on_fire` and fits the oak's own `hardy`.
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
    assert!(entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "hardy")
    ));
}

#[test]
fn new_form_passives_are_fitted_on_landing_tick_from_its_folded_stats() {
    let (mut app, _) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    let bark = buff_id(&app, "bark");
    utils::apply_buff(app.world_mut(), sapling, bark);
    utils::run_ticks(&mut app, 1);

    // The oak's sight is 1 plus the bark's 2: 3, at least the 2 `watchful`
    // holds on — but only once the bark is folded into the new form's stats.
    // Both passives are borne the tick the oak lands, not a tick later.
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS - 1);
    assert_eq!(entity_def::type_name(app.world(), sapling), "sapling");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "hardy")
    ));
    assert!(entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "watchful")
    ));
    // Folded the same tick: 0 armor, +1 from each.
    assert_eq!(
        entity_def::effective_stat(app.world(), sapling, EntityStatId::ARMOR),
        Some(FixedU64::from_num(2))
    );
}

#[test]
fn old_form_passive_does_not_fit_new_form_passives() {
    let (mut app, _) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let lookout = buff_id(&app, "lookout");
    assert!(entity_def::bears(app.world(), sapling, lookout));

    // The sapling's `lookout` would lift the oak's sight of 1 to 3, past the 2
    // `watchful` holds on; it goes before the oak's stats are folded, so the
    // oak lands with sight 1 and no `watchful`.
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(!entity_def::bears(app.world(), sapling, lookout));
    assert!(!entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "watchful")
    ));
}

#[test]
fn dropped_passive_stops_draining_on_landing_tick() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS - 1);
    // 40 after the wound, less 2 on the one burning tick.
    assert!(entity_def::bears(app.world(), sapling, on_fire));
    assert_eq!(utils::health_as_u32(&app, sapling), 38);

    // The oak lands with the fire gone from its folded stats, so the landing
    // tick's flow drains nothing: the pool carries its share, 38 of 100, onto
    // the oak's 100 and holds there — 38 × 100 / 100 = 38.
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
    assert_eq!(utils::health_as_u32(&app, sapling), 38);
}

#[test]
fn change_of_form_keeps_last_hit() {
    let (mut app, _) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    let (_, rival) = utils::create_owned(&mut app, "shed", 8, 8, 1);
    utils::run_ticks(&mut app, 1);

    // Hit a tick before the oak lands: the oak still counts the hit, so ten
    // ticks unhurt are not met on its landing tick.
    game_loop::damage::apply(
        app.world_mut(),
        rival,
        sapling,
        utils::fixed("1"),
        Slain::Remains,
    );
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(!requirements::met_by(
        app.world(),
        Some(0),
        sapling,
        &Requirement::UnhurtFor(10)
    ));
}

#[test]
fn concealment_of_new_form_passive_holds_on_landing_tick() {
    let (mut app, _) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The oak's `camouflage` is fitted at the landing, and the concealment
    // marker with it, the same tick.
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "camouflage")
    ));
    assert!(app.world().get::<ConcealedComponent>(sapling).is_some());
}

#[test]
fn zero_maximum_energy_carries_as_empty() {
    let (mut app, _) = app();
    let (sapling, sapling_id) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::order_morph(&mut app, sapling, "grove");
    utils::push_command(&mut app, PlayerCommand::CancelMorph { entity: sapling_id });

    // The seedling's `sapped` folds its 40 energy maximum to nothing.
    utils::run_ticks(&mut app, SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "seedling");
    assert_eq!(
        entity_def::effective_stat(app.world(), sapling, EntityStatId::MAX_ENERGY),
        Some(FixedU64::ZERO)
    );

    // Called off, the sapling takes back none of a zero maximum: empty, not
    // full.
    utils::run_ticks(&mut app, utils::APPLY - SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "sapling");
    assert_eq!(utils::energy_as_u32(&app, sapling), 0);
}

#[test]
fn new_entity_starts_full_of_effective_maximum() {
    let (mut app, on_fire) = app();
    // Twice the health for everything player 0 owns: a new sapling starts at
    // 200 of 200, not 100 of 200 under its fire line.
    app.world_mut()
        .resource_mut::<PlayerStats>()
        .add_entity_modifiers(
            0,
            EntityModifiers::PoolMaximums {
                modifiers: vec![EntityModifier {
                    stat: EntityStatId::MAX_HEALTH,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::ONE,
                }],
                pool_shift: PoolShift::Share,
            },
        );
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    assert_eq!(utils::health_as_u32(&app, sapling), 200);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
}

#[test]
fn change_of_form_carries_share_of_effective_maximum() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    let grown = buff_id(&app, "grown");
    utils::apply_buff(app.world_mut(), sapling, grown);
    utils::run_ticks(&mut app, 1);

    // 100 of a doubled 200 is a half, on the line and not under it. The oak
    // keeps the growth, so it lands at a half of its own doubled 200 — 100,
    // not a half of its base 100 — and does not catch fire.
    assert_eq!(utils::health_as_u32(&app, sapling), 100);
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert_eq!(utils::health_as_u32(&app, sapling), 100);
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
}

#[test]
fn applied_while_buff_outlives_change_of_form() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Set alight from outside before its own fire would catch: the copy is
    // an applied one, and a new form that names no fire keeps it burning
    // while the pool stays under the line.
    utils::wound(&mut app, sapling, "60");
    utils::apply_buff(app.world_mut(), sapling, on_fire);
    utils::run_ticks(&mut app, 1);
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(entity_def::bears(app.world(), sapling, on_fire));
}

#[test]
fn ignored_recast_of_passive_is_dropped_at_change_of_form() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), sapling, on_fire));

    // `on_fire` ignores a second application, so the cast leaves the
    // sapling's own fire as it was, and the oak, naming no fire, drops it.
    utils::apply_buff(app.world_mut(), sapling, on_fire);
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
}

#[test]
fn refreshed_passive_outlives_change_of_form() {
    let (mut app, _) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::run_ticks(&mut app, 1);
    let smolder = buff_id(&app, "smolder");
    assert!(entity_def::bears(app.world(), sapling, smolder));

    // `smolder` refreshes on a second application, so the cast replaces the
    // sapling's own copy with an applied one, which the oak keeps while the
    // pool stays under the line.
    utils::apply_buff(app.world_mut(), sapling, smolder);
    utils::order_morph(&mut app, sapling, "oak");
    utils::run_ticks(&mut app, OAK_LANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "oak");
    assert!(entity_def::bears(app.world(), sapling, smolder));
}

#[test]
fn interim_form_drops_old_passives() {
    let (mut app, on_fire) = app();
    let (sapling, _) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), sapling, on_fire));

    // Into the seedling on the way to the grove: still under the line, but
    // the seedling names no fire, only `hardy`.
    let hardy = buff_id(&app, "hardy");
    utils::order_morph(&mut app, sapling, "grove");
    utils::run_ticks(&mut app, SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "seedling");
    assert!(!entity_def::bears(app.world(), sapling, on_fire));
    assert!(entity_def::bears(app.world(), sapling, hardy));

    // The grove names `watchful` and not `hardy`: the one is fitted and the
    // other dropped the tick it lands.
    utils::run_ticks(&mut app, GROVE_LANDS - SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "grove");
    assert!(!entity_def::bears(app.world(), sapling, hardy));
    assert!(entity_def::bears(
        app.world(),
        sapling,
        buff_id(&app, "watchful")
    ));
}

#[test]
fn canceled_change_of_form_refits_origin_passives() {
    let (mut app, on_fire) = app();
    let (sapling, sapling_id) = utils::create_owned(&mut app, "sapling", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, sapling, "60");
    utils::order_morph(&mut app, sapling, "grove");
    utils::push_command(&mut app, PlayerCommand::CancelMorph { entity: sapling_id });
    utils::run_ticks(&mut app, SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "seedling");
    assert!(!entity_def::bears(app.world(), sapling, on_fire));

    // Called off as a seedling — the cancel reaches the simulation on tick
    // APPLY — it returns to the sapling, still under the line: its own fire
    // is fitted again the tick it returns.
    utils::run_ticks(&mut app, utils::APPLY - SEEDLING_STANDS);
    assert_eq!(entity_def::type_name(app.world(), sapling), "sapling");
    assert!(entity_def::bears(app.world(), sapling, on_fire));
}

#[test]
fn site_lapses_applied_while_buff() {
    let (mut app, on_fire) = app();
    let (shed, _) = utils::create_crewed_site(&mut app, "shed", 5, 5, 0);

    // Alight at full health: its requirement fails, and a site is refitted
    // for what lapses though it is fitted no passive.
    utils::apply_buff(app.world_mut(), shed, on_fire);
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), shed, on_fire));
}

#[test]
fn energy_line_passive_reads_energy_pool() {
    let (mut app, _) = app();
    let (troll, _) = utils::create_owned(&mut app, "troll", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // `spent` holds under a quarter of 40 energy: 9 < 10 after spending 31.
    let spent = buff_id(&app, "spent");
    assert!(!entity_def::bears(app.world(), troll, spent));
    assert_eq!(
        pools::spend(app.world_mut(), troll, PoolId::ENERGY, utils::fixed("31")),
        Spending::Paid
    );
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), troll, spent));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Ticks from a morph order into the oak to its landing: the declared 2.
const OAK_LANDS: u32 = 2;

/// Ticks from a morph order into the grove to the seedling standing: the
/// interim form is taken on the tick the order starts.
const SEEDLING_STANDS: u32 = 1;

/// Ticks from a morph order into the grove to its landing: 1 to the seedling
/// and the declared 8.
const GROVE_LANDS: u32 = 1 + 8;

/// One human player and one rival. Buffs: `on_fire` (health under a half →
/// `health_drain +2`), `smolder` (health under a half → `armor +1`,
/// refreshing), `lookout` (health at least 1 → `sight_range +2`), `lit`
/// (while the owner has a standing `shed`, `sight_range +1`), `spent` (energy
/// under a quarter, `energy_regen +1`), `embers` (health under a half →
/// `health_drain +1`, stacking to five), `hardy` (health at least 1 →
/// `armor +1`), `watchful` (sight at least 2 → `armor +1`), `camouflage`
/// (health at least 1 → conceal), `sapped` (health at least 1 →
/// `max_energy −100%`), `bark` (forever, `sight_range +2`), `grown` (forever,
/// `max_health +100%`, the pool clamped), `stunned` (forever, disables), `shadowmeld` (idle →
/// conceal), `finished` (built → `sight_range +1`), `scaffold` (unless built
/// → `sight_range +1`). Types: `shed` (100 health, bears `on_fire`), `troll` (100 health
/// regenerating 0.5, 40 energy, bears `on_fire` and `spent`), `lantern` (100
/// health, sight 8, bears `lit`), `kiln` (100 health, bears `embers`),
/// `sapling` (100 health, 40 energy, sight 1, bears `on_fire`, `smolder` and
/// `lookout`, becomes an `oak` in two ticks, or a `grove` through a
/// `seedling` in eight, which can be called off), `oak` (100 health, sight 1,
/// bears `hardy`, `watchful` and `camouflage`), `seedling` (100 health, 40
/// energy, bears `hardy` and `sapped`), `grove` (100 health, sight 2, bears
/// `watchful`), `huntress` (a mover bearing `shadowmeld`), `beacon` (100
/// health, sight 1, bears `finished` and `scaffold`).
fn app() -> (App, EntityBuffId) {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    let on_fire;
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        on_fire = registry.register_entity_buff(
            "on_fire",
            while_buff(
                Requirement::Health(Bound::Share(Threshold::Under(utils::fixed("0.5")))),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::HEALTH_DRAIN, "2"),
                ]))],
            ),
        );
        let lit = registry.register_entity_buff(
            "lit",
            while_buff(
                Requirement::EntityType("shed".to_string()),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::SIGHT_RANGE, "1"),
                ]))],
            ),
        );
        let spent = registry.register_entity_buff(
            "spent",
            while_buff(
                Requirement::Energy(Bound::Share(Threshold::Under(utils::fixed("0.25")))),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::ENERGY_REGEN, "1"),
                ]))],
            ),
        );
        let embers = registry.register_entity_buff(
            "embers",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::HEALTH_DRAIN, "1"),
                ]))],
                lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
                    utils::fixed("0.5"),
                )))),
                stack_rule: StackRule::StackToCap(5),
                interrupted_by: Vec::new(),
            },
        );
        let hardy = registry.register_entity_buff(
            "hardy",
            while_buff(
                Requirement::Health(Bound::Amount(Threshold::AtLeast(FixedU64::ONE))),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::ARMOR, "1"),
                ]))],
            ),
        );
        registry.register_entity_buff(
            "bark",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::SIGHT_RANGE, "2"),
                ]))],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        let watchful = registry.register_entity_buff(
            "watchful",
            while_buff(
                Requirement::Stat {
                    stat: EntityStatId::SIGHT_RANGE,
                    bound: Bound::Amount(Threshold::AtLeast(FixedU64::from_num(2))),
                },
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::ARMOR, "1"),
                ]))],
            ),
        );
        let lookout = registry.register_entity_buff(
            "lookout",
            while_buff(
                Requirement::Health(Bound::Amount(Threshold::AtLeast(FixedU64::ONE))),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::SIGHT_RANGE, "2"),
                ]))],
            ),
        );
        let smolder = registry.register_entity_buff(
            "smolder",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::ARMOR, "1"),
                ]))],
                lasting: Lasting::While(Requirement::Health(Bound::Share(Threshold::Under(
                    utils::fixed("0.5"),
                )))),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
        let camouflage = registry.register_entity_buff(
            "camouflage",
            while_buff(
                Requirement::Health(Bound::Amount(Threshold::AtLeast(FixedU64::ONE))),
                vec![EntityEffect::Conceal],
            ),
        );
        let sapped = registry.register_entity_buff(
            "sapped",
            while_buff(
                Requirement::Health(Bound::Amount(Threshold::AtLeast(FixedU64::ONE))),
                vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![EntityModifier {
                        stat: EntityStatId::MAX_ENERGY,
                        op: ModifierOp::PercentAdd,
                        magnitude: -FixedI64::ONE,
                    }],
                    pool_shift: PoolShift::Share,
                })],
            ),
        );
        registry.register_entity_buff(
            "grown",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![EntityModifier {
                        stat: EntityStatId::MAX_HEALTH,
                        op: ModifierOp::PercentAdd,
                        magnitude: FixedI64::ONE,
                    }],
                    pool_shift: PoolShift::Clamp,
                })],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register_entity_buff(
            "stunned",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        let shadowmeld = registry.register_entity_buff(
            "shadowmeld",
            while_buff(Requirement::Idle, vec![EntityEffect::Conceal]),
        );
        registry.register(
            EntityTypeDef::new("shed")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_build(4, utils::site_initial(&[PoolId::HEALTH]))
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_passives([on_fire]),
        );
        registry.register(
            EntityTypeDef::new("troll")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::builtin(
                    PoolId::HEALTH,
                    FixedU64::from_num(100),
                    utils::fixed("0.5"),
                    FixedU64::ZERO,
                    PoolInitial::Full,
                ))
                .with_pool(Pool::energy(40))
                .with_dying(2, [])
                .with_passives([on_fire, spent]),
        );
        registry.register(
            EntityTypeDef::new("lantern")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(8))
                .with_dying(2, [])
                .with_passives([lit]),
        );
        registry.register(
            EntityTypeDef::new("kiln")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_passives([embers]),
        );
        registry.register(
            EntityTypeDef::new("sapling")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::ONE)
                .with_pool(Pool::energy(40))
                .with_dying(2, [])
                .with_passives([on_fire, smolder, lookout])
                .with_morphs([
                    MorphTransition::new(
                        "oak",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(2),
                        MorphPlacement::Reserve,
                        MorphCancel::Committed,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "grove",
                        MorphCourse::via(
                            "seedling",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(8),
                        MorphPlacement::Reserve,
                        MorphCancel::Forfeit,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                ]),
        );
        registry.register(
            EntityTypeDef::new("seedling")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_build(4, utils::site_initial(&[PoolId::HEALTH, PoolId::ENERGY]))
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
                .with_pool(Pool::energy(40))
                .with_dying(2, [])
                .with_passives([hardy, sapped]),
        );
        let finished = registry.register_entity_buff(
            "finished",
            while_buff(
                Requirement::Built,
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::SIGHT_RANGE, "1"),
                ]))],
            ),
        );
        let scaffold = registry.register_entity_buff(
            "scaffold",
            while_buff(
                Requirement::Unless(Box::new(Requirement::Built)),
                vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![
                    utils::flat(EntityStatId::SIGHT_RANGE, "1"),
                ]))],
            ),
        );
        registry.register(
            EntityTypeDef::new("beacon")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_build(4, utils::site_initial(&[PoolId::HEALTH]))
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::ONE)
                .with_dying(2, [])
                .with_passives([finished, scaffold]),
        );
        registry.register(
            EntityTypeDef::new("grove")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(2))
                .with_dying(2, [])
                .with_passives([watchful]),
        );
        registry.register(
            EntityTypeDef::new("oak")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::ONE)
                .with_dying(2, [])
                .with_passives([hardy, watchful, camouflage]),
        );
        registry.register(
            EntityTypeDef::new("huntress")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    utils::fixed("0.5"),
                    utils::fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_dying(2, [])
                .with_passives([shadowmeld]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    (app, on_fire)
}

/// A buff held while `requirement` is met, with the given effects.
fn while_buff(requirement: Requirement, effects: Vec<EntityEffect>) -> EntityBuffDef {
    EntityBuffDef {
        effects,
        lasting: Lasting::While(requirement),
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// The id of the fixture's buff `name`.
fn buff_id(app: &App, name: &str) -> EntityBuffId {
    app.world()
        .resource::<ContentRegistry>()
        .entity_buff(name)
        .expect("the fixture registers the buff")
}
