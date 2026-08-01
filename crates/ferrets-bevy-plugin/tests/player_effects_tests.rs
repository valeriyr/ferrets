//! Player-wide effects: owner-wide unit modifiers reach every owned unit, and
//! player skills pay, boost, expire, and cool down.

mod utils;

use bevy::prelude::*;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    command::{PlayerCommand, SkillCasterRef},
    content::{
        entity_buffs::EntityBuffDef,
        entity_stats::EntityStatId,
        player_buffs::{PlayerBuffDef, PlayerBuffId},
        player_stats::PlayerStatId,
        registry::ContentRegistry,
        skills::{PlayerCastEffect, SkillCaster, SkillDef},
        stack_rule::StackRule,
        stats::{EntityModifier, ModifierOp, PlayerModifier},
    },
    game_loop,
    player_stats::PlayerStats,
    resources::Cost,
};

//
// ─── Owner-wide unit modifiers ───────────────────────────────────────────────
//

#[test]
fn unit_modifier_reaches_every_owned_unit() {
    let mut app = utils::player_effects_app();
    let (first, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);
    let (second, _) = utils::spawn_owned(&mut app, "runner", 7, 5, 0);
    let (enemy, _) = utils::spawn_owned(&mut app, "runner", 20, 20, 1);
    utils::run_ticks(&mut app, 1);
    let base = utils::effective_speed(&app, first);

    app.world_mut()
        .resource_mut::<PlayerStats>()
        .add_entity_modifier(0, speed_percent(1.0));
    utils::run_ticks(&mut app, 1);

    assert_eq!(utils::effective_speed(&app, first), base + base);
    assert_eq!(utils::effective_speed(&app, second), base + base);
    assert_eq!(
        utils::effective_speed(&app, enemy),
        base,
        "another player's units take nothing from the modifier"
    );
}

#[test]
fn unit_modifier_reaches_unit_spawned_after_it() {
    let mut app = utils::player_effects_app();
    let (veteran, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let base = utils::effective_speed(&app, veteran);

    app.world_mut()
        .resource_mut::<PlayerStats>()
        .add_entity_modifier(0, speed_percent(1.0));
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::effective_speed(&app, veteran), base + base);

    // A unit arriving after the modifier picks it up on its first recompute.
    let (recruit, _) = utils::spawn_owned(&mut app, "runner", 7, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::effective_speed(&app, recruit), base + base);
}

#[test]
fn buff_and_unit_modifier_fold_together() {
    let mut app = utils::player_effects_app();
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);

    let haste = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            "haste",
            EntityBuffDef {
                modifiers: vec![speed_percent(1.0)],
                duration: Some(5),
                stack_rule: StackRule::Refresh,
            },
        );
    game_loop::stats::apply_entity_buff(app.world_mut(), runner, haste);
    app.world_mut()
        .resource_mut::<PlayerStats>()
        .add_entity_modifier(0, speed_flat(0.5));
    utils::run_ticks(&mut app, 1);

    // (0.5 base + 0.5 flat) * (1 + 1.0 percent) = 2.
    assert_eq!(
        utils::effective_speed(&app, runner),
        FixedU64::from_num(2),
        "the buff and the owner-wide modifier fold into one snapshot"
    );
}

//
// ─── Player-level buffs ──────────────────────────────────────────────────────
//

#[test]
fn player_buff_mixes_both_arms() {
    let mut app = utils::player_effects_app();
    app.world_mut().resource_mut::<PlayerStats>().set_base(
        0,
        PlayerStatId::MAX_SUPPLY,
        FixedU64::from_num(10),
    );
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let base = utils::effective_speed(&app, runner);

    let rally = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_player_buff(
            "rally",
            PlayerBuffDef {
                player_modifiers: vec![PlayerModifier {
                    stat: PlayerStatId::MAX_SUPPLY,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(5),
                }],
                entity_modifiers: vec![speed_percent(1.0)],
                duration: Some(5),
                stack_rule: StackRule::Refresh,
            },
        );
    game_loop::stats::apply_player_buff(app.world_mut(), 0, rally);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        utils::effective_speed(&app, runner),
        base + base,
        "the entity arm reaches the owned unit"
    );
    assert_eq!(
        max_supply(&app),
        Some(FixedU64::from_num(15)),
        "the player arm lifts the owner's own ceiling"
    );

    utils::run_ticks(&mut app, 6);
    assert_eq!(
        utils::effective_speed(&app, runner),
        base,
        "the entity arm reverts when the buff expires"
    );
    assert_eq!(
        max_supply(&app),
        Some(FixedU64::from_num(10)),
        "the player arm reverts with it"
    );
}

//
// ─── Player skills ───────────────────────────────────────────────────────────
//

#[test]
fn player_skill_cast_boosts_then_expires() {
    let mut app = utils::player_effects_app();
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);
    utils::grant_gold(&mut app, 30);
    utils::run_ticks(&mut app, 1);
    let base = utils::effective_speed(&app, runner);
    let drums = app
        .world()
        .resource::<ContentRegistry>()
        .skill("drums")
        .expect("drums is registered");

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: drums,
            caster: SkillCasterRef::Player,
            target: None,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 20, "the cast pays its cost");
    assert_eq!(
        utils::effective_speed(&app, runner),
        base + base,
        "+100% speed while the cast lasts"
    );

    // A second cast during the cooldown is refused: nothing paid, nothing stacked.
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: drums,
            caster: SkillCasterRef::Player,
            target: None,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 20);
    assert_eq!(utils::effective_speed(&app, runner), base + base);

    // The cast runs out: speed reverts while the cooldown still runs.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::effective_speed(&app, runner), base);

    // Past the cooldown a fresh cast pays and boosts again.
    utils::run_ticks(&mut app, 10);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: drums,
            caster: SkillCasterRef::Player,
            target: None,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(utils::gold(app.world()), 10);
    assert_eq!(utils::effective_speed(&app, runner), base + base);
}

#[test]
fn unknown_player_skill_id_is_ignored() {
    let mut app = utils::player_effects_app();
    let (runner, _) = utils::spawn_owned(&mut app, "runner", 5, 5, 0);
    utils::grant_gold(&mut app, 30);
    utils::run_ticks(&mut app, 1);
    let base = utils::effective_speed(&app, runner);

    // A registry of its own mints an id one past anything the session's
    // registry ever handed out — the shape of a stale or hostile wire id.
    let mut foreign = ContentRegistry::default();
    let boost = foreign.register_player_buff(
        "boost",
        PlayerBuffDef {
            player_modifiers: Vec::new(),
            entity_modifiers: vec![speed_percent(1.0)],
            duration: Some(10),
            stack_rule: StackRule::Refresh,
        },
    );
    foreign.register_skill("first", free_skill(boost));
    let unknown = foreign.register_skill("second", free_skill(boost));

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: unknown,
            caster: SkillCasterRef::Player,
            target: None,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 1);

    assert_eq!(utils::gold(app.world()), 30, "nothing is paid");
    assert_eq!(
        utils::effective_speed(&app, runner),
        base,
        "nothing is laid over the player's units"
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────────
//

/// Player 0's effective supply ceiling.
fn max_supply(app: &App) -> Option<FixedU64> {
    app.world()
        .resource::<PlayerStats>()
        .effective(0, PlayerStatId::MAX_SUPPLY)
}

fn speed_flat(magnitude: f64) -> EntityModifier {
    EntityModifier {
        stat: EntityStatId::SPEED,
        op: ModifierOp::FlatAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

fn speed_percent(magnitude: f64) -> EntityModifier {
    EntityModifier {
        stat: EntityStatId::SPEED,
        op: ModifierOp::PercentAdd,
        magnitude: FixedI64::from_num(magnitude),
    }
}

/// A free skill whose only job is to make a foreign registry mint ids.
fn free_skill(buff: PlayerBuffId) -> SkillDef {
    SkillDef {
        cooldown: 10,
        caster: SkillCaster::Player {
            cost: Cost::new(),
            effect: PlayerCastEffect::ApplyBuff(buff),
        },
    }
}
