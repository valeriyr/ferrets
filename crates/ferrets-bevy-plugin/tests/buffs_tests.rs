//! Buff pipeline: a buff modifies effective stats for its duration, then reverts.

use ferrets_math::FixedI64;
use ferrets_simulation::{
    content::{
        buffs::{BuffDef, StackRule},
        registry::ContentRegistry,
        stats::{Modifier, ModifierOp, StatId},
    },
    game_loop, spawn,
};

mod utils;

//
// ─── Buff application and expiry ────────────────────────────────────────────
//

#[test]
fn buff_modifies_effective_stat_then_reverts_on_expiry() {
    let mut app = utils::combat_app();
    let (soldier, _) =
        spawn::spawn_entity(app.world_mut(), "soldier", utils::pos(5, 5), Some(0)).unwrap();

    let base = utils::effective_damage(&app, soldier);
    let frenzy = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_buff(
            "frenzy",
            BuffDef {
                modifiers: vec![Modifier {
                    stat: StatId::DAMAGE,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_num(1),
                }],
                duration: Some(3),
                stack_rule: StackRule::Refresh,
            },
        );

    // +100% damage for three ticks.
    game_loop::stats::apply_buff(app.world_mut(), soldier, frenzy);

    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_damage(&app, soldier),
        base + base,
        "a +100% buff doubles the effective stat this tick"
    );

    utils::run_ticks(&mut app, 2);
    assert_eq!(
        utils::effective_damage(&app, soldier),
        base + base,
        "the buff still applies on the last tick of its duration"
    );

    utils::run_ticks(&mut app, 1);
    assert_eq!(
        utils::effective_damage(&app, soldier),
        base,
        "the effective stat reverts the tick after the buff expires"
    );
}
