//! Skills: using a skill applies its effect and spends energy.

use bevy::prelude::*;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::{
    command::PlayerCommand,
    components::{
        buffs::{BuffDef, StackRule},
        energy::EnergyComponent,
        skills::{SkillDef, SkillEffect, SkillTarget},
        stats::{Modifier, ModifierOp, StatId},
    },
    content::{entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry},
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
            caster: mage_id,
            skill: battle_focus,
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
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// One human player and a `mage` that can cast a self-targeted +100% damage buff
/// costing 30 energy on a 5-tick cooldown.
fn app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let frenzy = registry.register_buff(
            "frenzy",
            BuffDef {
                modifiers: vec![Modifier {
                    stat: StatId::DAMAGE,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_num(1),
                }],
                duration: Some(20),
                stack_rule: StackRule::Refresh,
            },
        );
        let battle_focus = registry.register_skill(
            "battle_focus",
            SkillDef {
                cooldown: 5,
                energy_cost: FixedU64::from_num(30),
                target: SkillTarget::Caster,
                effect: SkillEffect::ApplyBuff(frenzy),
            },
        );
        registry.register(
            EntityTypeDef::new("mage")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(50)
                .with_attack(10, 1, 1, 4, 2)
                .with_energy(100, FixedU64::from_num(1))
                .with_skills([battle_focus]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

fn energy(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<EnergyComponent>(entity)
        .unwrap()
        .current()
}
