//! Bevy wiring for scripted scenarios: building a declared scene and running
//! the objectives and win/loss evaluation.
//!
//! A scenario runtime observes the same integer
//! [`GameView`](ferrets_script::ai::view::game::GameView) as an AI brain, but
//! instead of issuing commands it reports objective progress and whether the
//! game has been won or lost. Under the `Scripted` finish policy the
//! end-of-tick outcome check runs it on its cadence inside the deterministic
//! tick, publishes the latest progress in [`ScenarioObjectives`], and ends the
//! session when the script decides. Install [`ScenarioRuntimes`] at game
//! start; without it the check stands down and the game is unaffected.

use bevy::prelude::*;
use ferrets_script::scenario::{ObjectiveStatus, ScenarioRuntime};
use ferrets_simulation::resources::PlayerResources;
use ferrets_simulation::scenario::Scenario;
use ferrets_simulation::session::player_slot::PlayerId;

/// The live scenario evaluator and the player whose progress it judges. A
/// `NonSend` resource because a script runtime is single-threaded; absent
/// unless a scenario is running. The judged player is a fixed slot id (not
/// "whichever is local"), so the outcome is decided identically on every node.
pub struct ScenarioRuntimes {
    pub runtime: Box<dyn ScenarioRuntime>,
    pub player: PlayerId,
}

/// The latest objective progress, in declared order, for display. Refreshed on
/// each evaluation.
#[derive(Resource, Default)]
pub struct ScenarioObjectives(pub Vec<ObjectiveStatus>);

/// Installs the scenario runtime that judges `player`: the `NonSend` evaluator
/// and the empty progress resource. Call at game start.
pub fn install_scenario_runtime(
    world: &mut World,
    runtime: Box<dyn ScenarioRuntime>,
    player: PlayerId,
) {
    world.insert_non_send_resource(ScenarioRuntimes { runtime, player });
    world.init_resource::<ScenarioObjectives>();
}

/// Removes the scenario runtime and the progress resource. Call at game
/// teardown so a stale scenario never leaks into the next session.
pub fn remove_scenario_runtime(world: &mut World) {
    world.remove_non_send_resource::<ScenarioRuntimes>();
    world.remove_resource::<ScenarioObjectives>();
}

/// Builds the scenario's starting scene: its map with everything placed on it
/// (see [`instantiate_map`](crate::map::instantiate_map)), plus the
/// mission-specific stockpile.
pub fn instantiate_scenario(world: &mut World, scenario: &Scenario) {
    crate::map::instantiate_map(world, &scenario.map);

    let mut resources = world.resource_mut::<PlayerResources>();
    for stock in &scenario.stockpile {
        resources.add(stock.player, &stock.resource, stock.amount);
    }
}
