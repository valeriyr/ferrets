use bevy::prelude::*;
use ferrets_script::scenario::Outcome;
use ferrets_simulation::{
    game_loop,
    session::{GameResult, GameSession, Winner, ai_vision::AiVision, finish_policy::FinishPolicy},
};

use crate::{
    ai,
    scenario::{ScenarioObjectives, ScenarioRuntimes},
};

/// Applies the session's finish policy at the end of the tick, ending the game
/// once it is decided: the built-in last-standing check, a scripted scenario's
/// verdict, or nothing at all under `Endless`.
pub fn check_game_result(world: &mut World) {
    match world.resource::<GameSession>().finish_policy() {
        FinishPolicy::LastStanding { .. } => game_loop::game_result::check(world),
        FinishPolicy::Scripted => check_scenario(world),
        FinishPolicy::Endless => {}
    }
}

/// Evaluates the scenario on its cadence, publishes the objective progress in
/// [`ScenarioObjectives`], and ends the session on victory or defeat. Stands
/// down when no runtime is installed (an ordinary `Scripted` game, or replay
/// playback, where the replay is the sole authority), and skips evaluation
/// (with a log) when the judged slot is missing or unoccupied — a scenario
/// misconfiguration must not read as an instant empty-view defeat.
///
/// Reads the same committed integer view the AI observes. It advances at most
/// once per tick (the running group holds a blocked tick), so — unlike the AI
/// frame source — it needs no per-tick rerun guard. The judged player is fixed
/// by the runtime, keeping the outcome deterministic across nodes, and the
/// outcome is the shared verdict about that slot — victory names it the
/// winner. An evaluate error is logged and the game keeps running.
fn check_scenario(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();
    let Some(period) = world
        .get_non_send_resource::<ScenarioRuntimes>()
        .map(|runtimes| runtimes.runtime.period())
    else {
        return;
    };
    if !tick.is_multiple_of(period) {
        return;
    }

    // Take the runtime out so the view may borrow the world while the Lua state
    // is called into.
    let Some(mut runtimes) = world.remove_non_send_resource::<ScenarioRuntimes>() else {
        return;
    };
    let player = runtimes.player;
    let judged_race = world
        .resource::<GameSession>()
        .slot(player)
        .filter(|slot| slot.player_type().is_some())
        .map(|slot| slot.race().unwrap_or_default().to_string());
    match judged_race {
        Some(race) => {
            // The scenario judge evaluates win conditions over the whole game,
            // not as a competitor, so it sees everything.
            let view = ai::game_view(world, player, &race, AiVision::Omniscient);
            match runtimes.runtime.evaluate(&view) {
                Ok(status) => {
                    world.insert_resource(ScenarioObjectives(status.objectives));
                    let result = match status.outcome {
                        Outcome::Victory => Some(GameResult::Victory {
                            winner: Winner::Player(player),
                        }),
                        Outcome::Defeat => Some(GameResult::Defeat),
                        Outcome::Ongoing => None,
                    };
                    if let Some(result) = result {
                        world.resource_mut::<GameSession>().finish(result);
                    }
                }
                Err(error) => {
                    eprintln!("scenario evaluate failed at tick {tick}: {error}");
                }
            }
        }
        None => {
            eprintln!("the scenario judges slot {player}, which is missing or unoccupied");
        }
    }
    world.insert_non_send_resource(runtimes);
}
