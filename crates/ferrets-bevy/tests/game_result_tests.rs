//! Victory condition: under `LastStanding` the session ends when one player's
//! entities are all gone; under `Endless` it never ends on its own.

mod utils;

use ferrets_simulation::{
    command::PlayerCommand,
    session::{FinishPolicy, GameResult, GameSession},
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn last_standing_wins_when_the_only_opponent_is_destroyed() {
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
fn endless_never_finishes_even_when_a_player_is_wiped_out() {
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

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

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
