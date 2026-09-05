//! Buff pipeline: a buff modifies effective stats for its duration, then reverts.

use ferrets_content::{entity_stats::EntityStatId, stats::ModifierOp};
use ferrets_simulation::game_loop;

mod utils;

//
// ─── Buff application and expiry ────────────────────────────────────────────
//

#[test]
fn buff_modifies_effective_stat_then_reverts_on_expiry() {
    let mut app = utils::combat_app();
    let (soldier, _) =
        utils::create_entity(app.world_mut(), "soldier", utils::pos(5, 5), Some(0)).unwrap();

    let base = utils::effective_damage(&app, soldier);
    let frenzy = utils::register_entity_buff(
        &mut app,
        "frenzy",
        EntityStatId::DAMAGE,
        ModifierOp::PercentAdd,
        "1",
        Some(3),
    );

    // +100% damage for three ticks.
    game_loop::stats::apply_entity_buff(app.world_mut(), soldier, frenzy);

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
