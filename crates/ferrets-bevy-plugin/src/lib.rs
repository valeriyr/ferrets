//! Bevy integration for the ferrets simulation.
//!
//! # Setup
//!
//! ```ignore
//! use bevy::prelude::*;
//! use ferrets_bevy_plugin::SimulationPlugin;
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
//! supply_unmanned_input — idle frames for locally-sourced slots with no brain
//! supply_ai_input    — scripted AI frame source (thinks on its cadence)
//! flush_input        — drain PendingInput into InputFrames (runs while session is active)
//! command_executor   — translate InputFrames → OrderQueueComponent mutations
//! [ApplyDeferred]
//! process_dying      — exclusive system; advance Die orders, despawn entities that
//!                      finished dying
//! tick_orders        — exclusive system; full order lifecycle for alive entities:
//!                        prepare phase: flush cancelled entries, New → InProcessing,
//!                          Suspended → resumed, insert driver components
//!                        process phase: advance InProcessing front order, remove driver
//!                          components on finish, push chase sub-orders on suspend
//! process_pending_reveals — exclusive system; retry reappearing entities that finished
//!                      an order while boxed-in and still await a free cell
//! process_stats      — HP/mana regen, buff ticks                     [not yet implemented]
//! process_skills     — skill cooldown counters                        [not yet implemented]
//! process_entity_ai  — per-entity AI think (throttled, every N ticks) [not yet implemented]
//! check_game_result  — apply the finish policy; may end the session (e.g. last player standing)
//! tick_counter       — advance the simulation tick
//! ```
//!
//! Use `.after(SimulationSet)` to read sim state after the tick completes.

pub mod ai;
mod input;
pub mod network;
pub mod replay;
mod systems;

pub use ferrets_simulation::spawn;
pub use input::PendingInput;
pub use network::{
    BlockedStreak, ControlLinks, DesyncTracker, DropConfig, DropIntent, NetworkActive,
    NetworkPlugin, NetworkSession, PauseIntent, PendingPause, Stall, StallInfo, StallVotes,
    detect_drops, install_network_session, net_broadcast, net_checksum, net_control, net_receive,
};
pub use replay::{
    ReplayPlayback, ReplayPlugin, ReplayRecorder, install_replay_playback, install_replay_recorder,
    record_input, supply_replay_input, verify_replay_checksum,
};
pub use systems::flush_input;

use std::sync::Mutex;

use bevy::prelude::*;
use ferrets_simulation::{
    content::registry::ContentRegistry,
    entity_index::EntityIndex,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    map::Map,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, player_slot::PlayerSlot},
    simulation_id::SimulationIdGenerator,
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

fn session_is_not_paused(session: Res<GameSession>) -> bool {
    !session.is_paused()
}

/// Builds the input queue with the lockstep warmup pre-seeded: ticks
/// `0..SYNC_LATENCY` can never be targeted by a source scheduling `SYNC_LATENCY`
/// ahead, so every occupied slot is recorded idle for them — otherwise the loop
/// would block at startup. Unoccupied slots get nothing: no tick requires their
/// input. Seeded identically on every peer, so it stays deterministic.
fn warmup_input_frames(slots: &[PlayerSlot]) -> InputFrames {
    let mut frames = InputFrames::new(slots.len());
    for tick in 0..SYNC_LATENCY {
        for slot in slots {
            if slot.player_type().is_some() {
                frames.push_frame(PlayerFrame::idle(slot.id(), tick));
            }
        }
    }
    frames
}

/// (Re)sizes the per-player simulation resources to the session's slots. Call
/// at game start once [`GameSession`] holds the finalized configuration (e.g.
/// from a lobby), since the plugin is built before the real slots are known.
pub fn install_game_resources(world: &mut World) {
    let session = world.resource::<GameSession>();
    let player_count = session.slots().len();
    let frames = warmup_input_frames(session.slots());
    world.insert_resource(Selection::new(player_count));
    world.insert_resource(PlayerResources::new(player_count));
    world.insert_resource(frames);
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
        let frames = warmup_input_frames(session.slots());
        app.insert_resource(session)
            .insert_resource(map)
            .insert_resource(Selection::new(player_count))
            .insert_resource(PlayerResources::new(player_count))
            .insert_resource(frames)
            .init_resource::<ContentRegistry>()
            .init_resource::<EntityIndex>()
            .init_resource::<SimulationIdGenerator>()
            .init_resource::<PendingInput>()
            .add_systems(
                FixedUpdate,
                // flush_input and command_executor run whenever the session is active
                // (running OR blocked), so input keeps draining and a blocked tick can
                // notice its frame became ready and resume. command_executor sets the
                // blocked/running state from the frame's readiness.
                //
                // flush_input is the local player's frame source, so it is silenced
                // during replay playback — the replay drives every slot itself, and a
                // second local frame would collide with the recorded one.
                (
                    flush_input.run_if(not(resource_exists::<replay::ReplayPlayback>)),
                    systems::command_executor,
                )
                    .chain()
                    .in_set(SimulationSet)
                    .run_if(session_is_active.and(session_is_not_paused)),
            )
            .add_systems(
                FixedUpdate,
                // Entity processing and the tick counter advance only while running — a
                // blocked tick holds here, freezing the tick until the frame arrives.
                // ApplyDeferred makes command_executor's deferred mutations visible first.
                (
                    ApplyDeferred,
                    systems::process_dying,
                    systems::tick_orders,
                    systems::process_pending_reveals,
                    systems::check_game_result,
                    systems::tick_counter,
                )
                    .chain()
                    .in_set(SimulationSet)
                    .after(systems::command_executor)
                    .run_if(session_is_running.and(session_is_not_paused)),
            );
    }
}
