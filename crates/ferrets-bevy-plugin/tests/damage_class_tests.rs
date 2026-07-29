//! Armor & damage classes: bonus-vs-tag adds damage, flat armor subtracts it,
//! and a minimum-damage floor keeps a heavily-armored target killable.

use bevy::prelude::*;
use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::command::{PlayerCommand, SelectMode};
use ferrets_simulation::components::health::HealthComponent;
use ferrets_simulation::content::entity_type_def::EntityTypeDef;
use ferrets_simulation::content::location::Solidity;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::session::GameSession;
use ferrets_simulation::session::player_slot::PlayerSlot;
use ferrets_simulation::session::player_type::PlayerType;
use ferrets_simulation::simulation_id::SimulationId;
use ferrets_simulation::spawn;

mod utils;

//
// ─── Bonus-vs-tag and flat armor ────────────────────────────────────────────
//

#[test]
fn bonus_damage_vs_and_armor_shape_damage_per_hit() {
    let mut app = app();
    let (_, grunt) =
        spawn::spawn_entity(app.world_mut(), "grunt", utils::pos(5, 5), Some(0)).unwrap();
    let (tank, tank_id) =
        spawn::spawn_entity(app.world_mut(), "tank", utils::pos(6, 5), Some(1)).unwrap();

    attack(&mut app, grunt, tank_id);
    utils::run_ticks(&mut app, 15);

    // 10 base + 10 (vs armored) − 3 armor = 17 per hit, and three hits land in
    // 15 ticks on a 4-tick attack period.
    assert_eq!(
        200 - health(&app, tank),
        51,
        "expected three hits of 17 damage each"
    );
}

#[test]
fn armor_mitigates_and_never_makes_target_immune() {
    let mut app = app();
    // Two identical grunts attack in lockstep, so both land the same number of
    // hits over the same ticks.
    let (_, grunt_a) =
        spawn::spawn_entity(app.world_mut(), "grunt", utils::pos(5, 10), Some(0)).unwrap();
    let (scout, scout_id) =
        spawn::spawn_entity(app.world_mut(), "scout", utils::pos(6, 10), Some(1)).unwrap();
    let (_, grunt_b) =
        spawn::spawn_entity(app.world_mut(), "grunt", utils::pos(5, 15), Some(0)).unwrap();
    let (fortress, fortress_id) =
        spawn::spawn_entity(app.world_mut(), "fortress", utils::pos(6, 15), Some(1)).unwrap();

    attack(&mut app, grunt_a, scout_id);
    attack(&mut app, grunt_b, fortress_id);
    utils::run_ticks(&mut app, 15);

    // Both grunts land three hits. The scout is untagged and takes the full
    // 10/hit; the fortress's armor (100) exceeds the grunt's damage, so the floor
    // leaves exactly 1/hit — mitigated tenfold, yet never immune.
    assert_eq!(
        (200 - health(&app, scout), 200 - health(&app, fortress)),
        (30, 3),
        "expected the scout at 10/hit and the fortress at the 1/hit floor"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Two players and a roster with an anti-armor `grunt`, an `armored` `tank`, a
/// plain `scout`, and a `fortress` whose armor exceeds the grunt's damage.
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_tag("armored");
        registry.register(
            EntityTypeDef::new("grunt")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(50)
                .with_attack(10, 1, 1, 4, 2)
                .with_bonus_damage_vs([("armored", 10u32)]),
        );
        registry.register(
            EntityTypeDef::new("tank")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(200)
                .with_armor(3)
                .with_tags(["armored"]),
        );
        registry.register(
            EntityTypeDef::new("scout")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(200),
        );
        registry.register(
            EntityTypeDef::new("fortress")
                .with_location(utils::GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(200)
                .with_armor(100),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

fn health(app: &App, entity: Entity) -> u32 {
    app.world()
        .get::<HealthComponent>(entity)
        .unwrap()
        .current()
        .to_num::<u32>()
}

fn attack(app: &mut App, attacker: SimulationId, target: SimulationId) {
    utils::push_command(
        app,
        PlayerCommand::SelectById {
            id: attacker,
            mode: SelectMode::Replace,
        },
    );
    utils::push_command(
        app,
        PlayerCommand::Attack {
            target,
            flush: true,
        },
    );
}
