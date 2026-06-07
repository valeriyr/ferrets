//! Bevy integration for the ferrets simulation.
//!
//! # Setup
//!
//! ```ignore
//! use bevy::prelude::*;
//! use ferrets_bevy::SimulationPlugin;
//! use ferrets_simulation::{map::Map, session::GameSession};
//!
//! App::new()
//!     .add_plugins(MinimalPlugins)
//!     .add_plugins(SimulationPlugin::new(session, map))
//!     .run();
//! ```
//!
//! # Ordering
//!
//! All simulation systems run in `FixedUpdate` inside [`SimulationSet`].
//! The order mirrors a classic RTS tick loop:
//!
//! ```text
//! flush_input        — drain PendingInput into InputFrames (runs while session is active)
//! command_executor   — translate InputFrames → OrderQueueComponent mutations
//! [ApplyDeferred]
//! tick_orders        — exclusive system; full order lifecycle in one entry point:
//!                        prepare phase: flush cancelled entries, New → InProcessing, insert driver components
//!                        process phase: advance InProcessing front order, remove driver components on finish
//! process_dying      — tick dying entities                           [not yet implemented]
//! process_stats      — HP/mana regen, buff ticks                     [not yet implemented]
//! process_skills     — skill cooldown counters                        [not yet implemented]
//! process_entity_ai  — per-entity AI think (throttled, every N ticks) [not yet implemented]
//! check_game_result  — evaluate victory conditions                    [not yet implemented]
//! tick_counter       — advance the simulation tick
//! ```
//!
//! Use `.after(SimulationSet)` to read sim state after the tick completes.
mod input;
mod systems;

pub use ferrets_simulation::spawn::spawn_entity;
pub use input::PendingInput;
pub use systems::flush_input;

use std::sync::Mutex;

use bevy::prelude::*;
use ferrets_simulation::{
    content::registry::ContentRegistry, input::InputFrames, map::Map, selection::Selection,
    session::GameSession, simulation_id::SimulationIdGenerator,
};

/// System set containing all simulation systems.
///
/// Schedule systems that read sim state `.after(SimulationSet)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimulationSet;

fn session_is_active(session: Res<GameSession>) -> bool {
    session.is_active()
}

fn session_is_running(session: Res<GameSession>) -> bool {
    session.is_running()
}

/// Drives the ferrets simulation from Bevy's `FixedUpdate` schedule.
///
/// Inserts all simulation resources and registers simulation systems.
/// Requires `MinimalPlugins` (or `DefaultPlugins`) alongside it.
pub struct SimulationPlugin(Mutex<Option<(GameSession, Map)>>);

impl SimulationPlugin {
    pub fn new(session: GameSession, map: Map) -> Self {
        Self(Mutex::new(Some((session, map))))
    }
}

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        let (session, map) = self
            .0
            .lock()
            .unwrap()
            .take()
            .expect("SimulationPlugin::build called twice");

        let player_count = session.slots().len();
        app.insert_resource(session)
            .insert_resource(map)
            .insert_resource(Selection::new(player_count))
            .insert_resource(InputFrames::new(player_count))
            .init_resource::<ContentRegistry>()
            .init_resource::<SimulationIdGenerator>()
            .init_resource::<PendingInput>()
            .add_systems(
                FixedUpdate,
                // flush_input runs whenever the session is active so commands are always
                // drained into InputFrames, even while the sim is blocked waiting for peers.
                flush_input.in_set(SimulationSet).run_if(session_is_active),
            )
            .add_systems(
                FixedUpdate,
                // All entity-processing systems run only when the session is fully running.
                // ApplyDeferred between command_executor and tick_orders ensures any deferred
                // world mutations from command_executor are visible before order processing.
                (
                    systems::command_executor,
                    ApplyDeferred,
                    systems::tick_orders,
                    systems::tick_counter,
                )
                    .chain()
                    .in_set(SimulationSet)
                    .after(flush_input)
                    .run_if(session_is_running),
            );
    }
}
