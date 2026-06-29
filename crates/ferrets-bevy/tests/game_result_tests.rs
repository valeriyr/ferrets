//! Victory condition: under `LastStanding` the session ends when one player's
//! entities are all gone; under `Endless` it never ends on its own.

mod utils;

use ferrets_math::FixedU64;
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::{
    command::PlayerCommand,
    components::location::Solidity,
    content::{entity_type_def::EntityTypeDef, registry::ContentRegistry},
    session::{
        FinishPolicy, GameResult, GameSession, player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn,
};

use utils::GROUND;

#[test]
fn last_standing_wins_when_only_opponent_is_destroyed() {
    let mut app = utils::orders_app();
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);

    let world = app.world_mut();
    let (_, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, enemy_id) = spawn::spawn_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();

    // Both players still field a unit, so the game is in progress.
    utils::run_ticks(&mut app, 1);
    assert_eq!(app.world().resource::<GameSession>().result(), None);

    // Player 1's only unit is chased down, killed, and despawned — player 0 wins.
    attack(&mut app, attacker_id, enemy_id, 18);
    assert_eq!(
        app.world().resource::<GameSession>().result(),
        Some(GameResult::Victory { winner: 0 }),
    );
}

#[test]
fn endless_never_finishes_even_when_player_is_wiped_out() {
    // orders_app uses the Endless policy: destroying every opposing unit must not
    // end the game.
    let mut app = utils::orders_app();
    let world = app.world_mut();
    let (_, attacker_id) =
        spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, enemy_id) = spawn::spawn_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();

    attack(&mut app, attacker_id, enemy_id, 18);

    assert_eq!(app.world().resource::<GameSession>().result(), None);
}

#[test]
fn dropped_player_is_excluded_so_game_resolves_among_rest() {
    let mut app = three_player_soldier_app();
    let world = app.world_mut();
    let (_, p0) = spawn::spawn_entity(world, "soldier", utils::pos(5, 5), Some(0)).unwrap();
    let (_, p1) = spawn::spawn_entity(world, "soldier", utils::pos(7, 5), Some(1)).unwrap();
    // Player 2's unit sits far away and never moves.
    spawn::spawn_entity(world, "soldier", utils::pos(20, 20), Some(2)).unwrap();

    // All three present → in progress.
    utils::run_ticks(&mut app, 1);
    assert_eq!(app.world().resource::<GameSession>().result(), None);

    // Player 2 drops. Two players still field units, so the game continues —
    // player 2's lingering unit does not keep the game alive.
    app.world_mut().resource_mut::<GameSession>().drop_player(2);
    utils::run_ticks(&mut app, 1);
    assert_eq!(app.world().resource::<GameSession>().result(), None);

    // Player 0 wipes player 1. Only player 0 (non-dropped) survives → it wins,
    // even though player 2's idle unit is still on the map.
    attack(&mut app, p0, p1, 18);
    assert_eq!(
        app.world().resource::<GameSession>().result(),
        Some(GameResult::Victory { winner: 0 }),
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A 3-player `LastStanding` app with a minimal `soldier` roster, started.
fn three_player_soldier_app() -> bevy::prelude::App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None),
        PlayerSlot::occupied(1, PlayerType::Human, None),
        PlayerSlot::occupied(2, PlayerType::Human, None),
    ]);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 2, 2),
        );
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// Issues an attack from `attacker` onto `target` and runs `ticks` ticks.
fn attack(app: &mut bevy::prelude::App, attacker: SimulationId, target: SimulationId, ticks: u32) {
    utils::push_command(app, PlayerCommand::SelectById { id: attacker });
    utils::push_command(
        app,
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
    utils::run_ticks(app, ticks);
}
