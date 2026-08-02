//! Entity skills: using a skill applies its effect and pays its costs.

use bevy::prelude::*;
use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::{
    command::{PlayerCommand, SkillCasterRef},
    components::energy::EnergyComponent,
    content::{
        entity_stats::EntityStatId,
        research::ResearchDef,
        skills::{
            EntityCastCost, EntityCastEffect, EntityCastTarget, SkillCaster, SkillDef, SkillId,
        },
        stats::ModifierOp,
        {entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry},
    },
    player_research::PlayerResearch,
    resources::{self, Cost},
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
    spawn,
};

mod utils;

//
// ─── Skill use and energy cost ──────────────────────────────────────────────
//

#[test]
fn using_skill_applies_effect_and_spends_energy() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    let battle_focus = app
        .world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("skill defined");
    let base = utils::effective_damage(&app, mage);

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: battle_focus,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the self-buff skill doubles the mage's damage"
    );
    // 100 full − 30 cost, then +1 regen on the cast tick and each of the two
    // after it: regen runs after the spend within the same tick.
    assert_eq!(
        energy(&app, mage),
        FixedU64::from_num(73),
        "the cast spent exactly its 30-point cost"
    );
}

//
// ─── Resource and health costs ──────────────────────────────────────────────
//

#[test]
fn skill_with_resource_cost_pays_stockpile() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    utils::grant_gold(&mut app, 30);

    let rally = skill(&app, "rally");
    let base = utils::effective_damage(&app, mage);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: rally,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the cast applied its buff"
    );
    assert_eq!(
        utils::gold(app.world()),
        5,
        "the cast spent exactly its 25-gold cost"
    );
}

#[test]
fn skill_with_unaffordable_resource_cost_is_refused() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    utils::grant_gold(&mut app, 10);

    let rally = skill(&app, "rally");
    let base = utils::effective_damage(&app, mage);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: rally,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base,
        "the refused cast applied nothing"
    );
    assert_eq!(
        utils::gold(app.world()),
        10,
        "the refused cast paid nothing"
    );
}

#[test]
fn skill_with_health_cost_pays_health() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    let sacrifice = skill(&app, "sacrifice");
    let base = utils::effective_damage(&app, mage);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: sacrifice,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the cast applied its buff"
    );
    assert_eq!(
        utils::health(&app, mage),
        40,
        "the cast paid exactly its 10-health cost"
    );
}

#[test]
fn skill_with_lethal_health_cost_is_refused() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    // The cost equals the mage's full health: surviving on zero is not
    // surviving, so the cast is refused.
    let last_rite = skill(&app, "last_rite");
    let base = utils::effective_damage(&app, mage);
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: last_rite,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base,
        "the refused cast applied nothing"
    );
    assert_eq!(
        utils::health(&app, mage),
        50,
        "the refused cast paid nothing"
    );
}

//
// ─── Requirements ───────────────────────────────────────────────────────────
//

#[test]
fn skill_requirement_gates_cast() {
    let mut app = app();
    let (mage, mage_id) =
        spawn::spawn_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    let war_secret = skill(&app, "war_secret");
    let base = utils::effective_damage(&app, mage);

    // Before the research, the cast is refused outright.
    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: war_secret,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::effective_damage(&app, mage),
        base,
        "the gated cast applied nothing"
    );

    // Completing the research unlocks the same command.
    let arcana = app
        .world()
        .resource::<ContentRegistry>()
        .research("arcana")
        .expect("research defined");
    app.world_mut()
        .resource_mut::<PlayerResearch>()
        .complete(0, arcana);

    utils::push_command(
        &mut app,
        PlayerCommand::UseSkill {
            skill: war_secret,
            caster: SkillCasterRef::Entity(mage_id),
            target: None,
        },
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the unlocked cast applied its buff"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player and a `mage` whose self-targeted +100% damage buff can be
/// cast four ways: `battle_focus` (30 energy), `rally` (25 gold), `sacrifice`
/// (10 health), and `last_rite` (its whole 50 health) — plus the free
/// `war_secret`, gated on the `arcana` research.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    app.world_mut()
        .resource_mut::<ContentRegistry>()
        .register_resource("gold");
    let frenzy = utils::register_entity_buff(
        &mut app,
        "frenzy",
        EntityStatId::DAMAGE,
        ModifierOp::PercentAdd,
        1.0,
        Some(20),
    );
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let costed = |costs| SkillDef {
            cooldown: 5,
            caster: SkillCaster::Entity {
                costs,
                target: EntityCastTarget::Caster,
                effect: EntityCastEffect::ApplyBuff(frenzy),
            },
            requires: Vec::new(),
        };
        let battle_focus = registry.register_skill(
            "battle_focus",
            costed(vec![EntityCastCost::Energy(FixedU64::from_num(30))]),
        );
        let rally = registry.register_skill(
            "rally",
            costed(vec![EntityCastCost::Resources(resources::cost([(
                "gold", 25,
            )]))]),
        );
        let sacrifice = registry.register_skill(
            "sacrifice",
            costed(vec![EntityCastCost::Health(FixedU64::from_num(10))]),
        );
        let last_rite = registry.register_skill(
            "last_rite",
            costed(vec![EntityCastCost::Health(FixedU64::from_num(50))]),
        );
        registry.register_research(
            "arcana",
            ResearchDef::new(Cost::new(), 5, None, Vec::<String>::new()),
        );
        let war_secret = registry.register_skill(
            "war_secret",
            SkillDef {
                requires: vec!["arcana".to_string()],
                ..costed(Vec::new())
            },
        );
        registry.register(
            EntityTypeDef::new("mage")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_attack(10, 1, 1, 4, 2)
                .with_energy(100, FixedU64::from_num(1))
                .with_skills([battle_focus, rally, sacrifice, last_rite, war_secret]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// The registered id of `name`.
fn skill(app: &App, name: &str) -> SkillId {
    app.world()
        .resource::<ContentRegistry>()
        .skill(name)
        .expect("skill defined")
}

fn energy(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<EnergyComponent>(entity)
        .unwrap()
        .current()
}
