//! Entity skills: using a skill applies its effect and pays its costs.

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    cost::Cost,
    detection::Detection,
    entity_buffs::EntityBuffId,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    kinds::{Kind, Kinds},
    location::Solidity,
    pool::Pool,
    pool_def::PoolId,
    price::{self, Price},
    registry::ContentRegistry,
    requirement::Requirement,
    research::ResearchDef,
    skills::{Casting, EntityCastEffect, EntityCastTarget, Reach, SkillCaster, SkillDef},
    stats::ModifierOp,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{SkillCasterRef, SkillTarget},
    components::entity_skills::SkillsComponent,
    entity_def,
    game_loop::{
        self,
        buffs::Bearing,
        cast::{self, AimRefusal, CastAim},
    },
    player_research::PlayerResearch,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
};

mod utils;

//
// ─── Skill use and energy cost ──────────────────────────────────────────────
//

#[test]
fn using_skill_applies_effect_and_spends_energy() {
    let mut app = app();
    let (mage, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let base = utils::effective_damage(&app, mage);

    utils::use_skill(
        &mut app,
        "battle_focus",
        SkillCasterRef::Entity(mage_id),
        None,
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
        utils::energy_as_u32(&app, mage),
        73,
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
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    utils::grant_gold(&mut app, 30);
    let base = utils::effective_damage(&app, mage);
    utils::use_skill(&mut app, "rally", SkillCasterRef::Entity(mage_id), None);
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
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    utils::grant_gold(&mut app, 10);
    let base = utils::effective_damage(&app, mage);
    utils::use_skill(&mut app, "rally", SkillCasterRef::Entity(mage_id), None);
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
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let base = utils::effective_damage(&app, mage);
    utils::use_skill(&mut app, "sacrifice", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the cast applied its buff"
    );
    assert_eq!(
        utils::health_as_u32(&app, mage),
        40,
        "the cast paid exactly its 10-health cost"
    );
}

#[test]
fn skill_with_lethal_health_cost_is_refused() {
    let mut app = app();
    let (mage, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    // The cost equals the mage's full health: surviving on zero is not
    // surviving, so the cast is refused.
    let base = utils::effective_damage(&app, mage);
    utils::use_skill(&mut app, "last_rite", SkillCasterRef::Entity(mage_id), None);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        utils::effective_damage(&app, mage),
        base,
        "the refused cast applied nothing"
    );
    assert_eq!(
        utils::health_as_u32(&app, mage),
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
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let base = utils::effective_damage(&app, mage);

    // Before the research, the cast is refused outright.
    utils::use_skill(
        &mut app,
        "war_secret",
        SkillCasterRef::Entity(mage_id),
        None,
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
        .mark_completed(0, arcana);

    utils::use_skill(
        &mut app,
        "war_secret",
        SkillCasterRef::Entity(mage_id),
        None,
    );
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        utils::effective_damage(&app, mage),
        base + base,
        "the unlocked cast applied its buff"
    );
}

//
// ─── What a buff can land on ────────────────────────────────────────────────
//

#[test]
fn buff_cast_on_target_lacking_its_stat_is_refused_and_costs_nothing() {
    let mut app = app();
    let (mage, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (totem, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(7, 5), Some(0)).unwrap();

    // The totem has no damage for frenzy to move: no buff lands, the 10
    // energy stays in the mage's full 100, and the skill is still ready.
    utils::use_skill(
        &mut app,
        "empower",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Entity(totem_id)),
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    let frenzy = frenzy(&app);
    assert!(!entity_def::bears(app.world(), totem, frenzy));
    assert_eq!(utils::energy_as_u32(&app, mage), 100);
    let empower = utils::skill_id(&app, "empower");
    assert!(
        app.world()
            .get::<SkillsComponent>(mage)
            .unwrap()
            .ready(empower)
    );
}

#[test]
fn buff_cast_on_site_meeting_its_terms_lands() {
    let mut app = app();
    let (_, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (site, site_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(7, 5), Some(0)).unwrap();
    utils::mark_as_site(app.world_mut(), site);

    utils::use_skill(
        &mut app,
        "empower",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Entity(site_id)),
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(entity_def::bears(app.world(), site, frenzy(&app)));
}

#[test]
fn buff_cast_on_target_carrying_its_stat_lands() {
    let mut app = app();
    let (_, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (other, other_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(7, 5), Some(0)).unwrap();

    utils::use_skill(
        &mut app,
        "empower",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Entity(other_id)),
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert!(entity_def::bears(app.world(), other, frenzy(&app)));
}

#[test]
fn applying_buff_to_entity_lacking_its_stat_is_uncarried() {
    let mut app = app();
    let (totem, _) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(7, 5), Some(0)).unwrap();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let frenzy = frenzy(&app);

    assert_eq!(
        game_loop::buffs::apply_entity_buff(app.world_mut(), totem, frenzy),
        Bearing::Uncarried
    );
    assert!(!entity_def::bears(app.world(), totem, frenzy));
    assert_eq!(
        game_loop::buffs::apply_entity_buff(app.world_mut(), mage, frenzy),
        Bearing::Borne
    );
}

//
// ─── Heals ───────────────────────────────────────────────────────────────
//

#[test]
fn heal_restores_up_to_maximum() {
    let mut app = app();
    let (_, mage_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (totem, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(7, 5), Some(0)).unwrap();
    utils::wound(&mut app, totem, "5");

    // 45 + 10 = 55, held at the totem's 50.
    utils::use_skill(
        &mut app,
        "mend",
        SkillCasterRef::Entity(mage_id),
        Some(SkillTarget::Entity(totem_id)),
    );
    utils::run_ticks(&mut app, utils::APPLY + 2);
    assert_eq!(utils::health_as_u32(&app, totem), 50);
}

//
// ─── Why an aim is refused ───────────────────────────────────────────────
//

#[test]
fn aim_without_target_is_unaimed() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    assert_eq!(aim(&app, mage, "empower", None), Err(AimRefusal::Unaimed));
}

#[test]
fn aim_at_cell_off_map_is_off_map() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();

    // The map is 32 cells a side: cell (40, 5) lies past its edge, (7, 5) on it.
    assert_eq!(
        aim(
            &app,
            mage,
            "sweep",
            Some(SkillTarget::Position(utils::pos(40, 5)))
        ),
        Err(AimRefusal::OffMap)
    );
    assert_eq!(
        aim(
            &app,
            mage,
            "sweep",
            Some(SkillTarget::Position(utils::pos(7, 5)))
        ),
        Ok(CastAim::Cell(CellPos::new(7, 5)))
    );
}

#[test]
fn aim_at_target_out_of_sight_is_unseen() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (_, stray_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(25, 25), None).unwrap();
    utils::run_ticks(&mut app, 1);

    // 20 cells off, past the mage's sight of 4.
    assert_eq!(
        aim(&app, mage, "empower", Some(SkillTarget::Entity(stray_id))),
        Err(AimRefusal::Unseen)
    );
}

#[test]
fn aim_at_ownerless_target_of_own_only_skill_is_wrong_side() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (_, stray_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(7, 5), None).unwrap();
    // One tick for the fog to make the stray out: none stands seen before it.
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        aim(&app, mage, "bless", Some(SkillTarget::Entity(stray_id))),
        Err(AimRefusal::WrongSide)
    );
}

#[test]
fn aim_at_type_skill_does_not_name_is_wrong_kind() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (_, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(7, 5), Some(0)).unwrap();

    assert_eq!(
        aim(&app, mage, "bless", Some(SkillTarget::Entity(totem_id))),
        Err(AimRefusal::WrongKind)
    );
}

#[test]
fn aim_at_target_lacking_buff_stat_is_uncarried() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (_, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(7, 5), Some(0)).unwrap();

    assert_eq!(
        aim(&app, mage, "empower", Some(SkillTarget::Entity(totem_id))),
        Err(AimRefusal::Uncarried)
    );
}

#[test]
fn aim_heal_at_target_without_health_is_uncarried() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (_, banner_id) =
        utils::create_entity(app.world_mut(), "banner", utils::pos(7, 5), Some(0)).unwrap();
    let (totem, totem_id) =
        utils::create_entity(app.world_mut(), "totem", utils::pos(5, 7), Some(0)).unwrap();

    assert_eq!(
        aim(&app, mage, "mend", Some(SkillTarget::Entity(banner_id))),
        Err(AimRefusal::Uncarried)
    );
    assert_eq!(
        aim(&app, mage, "mend", Some(SkillTarget::Entity(totem_id))),
        Ok(CastAim::Entity(totem))
    );
}

#[test]
fn aim_at_own_named_carrier_is_entity() {
    let mut app = app();
    let (mage, _) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(5, 5), Some(0)).unwrap();
    let (other, other_id) =
        utils::create_entity(app.world_mut(), "mage", utils::pos(7, 5), Some(0)).unwrap();

    assert_eq!(
        aim(&app, mage, "bless", Some(SkillTarget::Entity(other_id))),
        Ok(CastAim::Entity(other))
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player and a `mage`, sight 4, whose self-targeted +100% damage buff can be
/// cast four ways: `battle_focus` (30 energy), `rally` (25 gold), `sacrifice`
/// (10 health), and `last_rite` (its whole 50 health) — plus the free
/// `war_secret`, gated on the `arcana` research, and `empower` (10 energy),
/// the same buff cast on another of anyone's, and `bless`, the same cast on
/// another of the caster's own mages, `mend`, a 10-point heal on anyone's,
/// and `sweep`, a watch over a cell. A `totem` stands with health and no damage, a `banner` with
/// neither.
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
        "1",
        Some(20),
    );
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let costed = |costs| SkillDef {
            cooldown: 5,
            caster: SkillCaster::Entity {
                costs,
                target: EntityCastTarget::Caster,
                reach: Reach::Wherever,
                casting: Casting::Instant,
                effect: EntityCastEffect::ApplyBuff(frenzy),
            },
            requires: Vec::new(),
        };
        let battle_focus = registry.register_skill(
            "battle_focus",
            costed(vec![Cost::Energy(FixedU64::from_num(30))]),
        );
        let rally = registry.register_skill(
            "rally",
            costed(vec![Cost::Resources(price::from([("gold", 25)]))]),
        );
        let sacrifice = registry.register_skill(
            "sacrifice",
            costed(vec![Cost::Health(FixedU64::from_num(10))]),
        );
        let last_rite = registry.register_skill(
            "last_rite",
            costed(vec![Cost::Health(FixedU64::from_num(50))]),
        );
        let arcana = registry.register_research(
            "arcana",
            ResearchDef::new(Price::new(), 5, None, Vec::new()),
        );
        let empower = registry.register_skill(
            "empower",
            SkillDef {
                cooldown: 5,
                caster: SkillCaster::Entity {
                    costs: vec![Cost::Energy(FixedU64::from_num(10))],
                    target: EntityCastTarget::Standing {
                        side: Affiliation::Anyone,
                        kinds: Kinds::Any,
                    },
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::ApplyBuff(frenzy),
                },
                requires: Vec::new(),
            },
        );
        let bless = registry.register_skill(
            "bless",
            SkillDef {
                cooldown: 5,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Standing {
                        side: Affiliation::Own,
                        kinds: Kinds::only([Kind::Type("mage".to_string())]),
                    },
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::ApplyBuff(frenzy),
                },
                requires: Vec::new(),
            },
        );
        let mend = registry.register_skill(
            "mend",
            SkillDef {
                cooldown: 5,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Standing {
                        side: Affiliation::Anyone,
                        kinds: Kinds::Any,
                    },
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Heal(FixedU64::from_num(10)),
                },
                requires: Vec::new(),
            },
        );
        let sweep = registry.register_skill(
            "sweep",
            SkillDef {
                cooldown: 5,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Position,
                    reach: Reach::Wherever,
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Watch {
                        radius: 2,
                        duration: 5,
                        detection: Detection::Blind,
                    },
                },
                requires: Vec::new(),
            },
        );
        let war_secret = registry.register_skill(
            "war_secret",
            SkillDef {
                requires: vec![Requirement::Research(arcana)],
                ..costed(Vec::new())
            },
        );
        registry.register(
            EntityTypeDef::new("mage")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(50))
                .with_attack(utils::weapon(utils::GROUND), 10, 1, 1, 4, 2)
                .with_stat(EntityStatId::SIGHT_RANGE, FixedU64::from_num(4))
                .with_pool(Pool::builtin(
                    PoolId::ENERGY,
                    FixedU64::from_num(100),
                    FixedU64::from_num(1),
                    FixedU64::ZERO,
                ))
                .with_skills([
                    battle_focus,
                    rally,
                    sacrifice,
                    last_rite,
                    war_secret,
                    empower,
                    bless,
                    mend,
                    sweep,
                ]),
        );
        registry.register(
            EntityTypeDef::new("totem")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(50)),
        );
        registry.register(EntityTypeDef::new("banner").with_location(
            utils::GROUND,
            CellSize::ONE,
            Solidity::Solid,
        ));
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// The fixture's damage buff.
fn frenzy(app: &App) -> EntityBuffId {
    app.world()
        .resource::<ContentRegistry>()
        .entity_buff("frenzy")
        .expect("the fixture registers frenzy")
}

/// What `cast::aim` makes of player 0's `caster` aiming `skill` at `target`.
fn aim(
    app: &App,
    caster: Entity,
    skill: &str,
    target: Option<SkillTarget>,
) -> Result<CastAim, AimRefusal> {
    cast::aim(app.world(), 0, caster, utils::skill_id(app, skill), target)
}
