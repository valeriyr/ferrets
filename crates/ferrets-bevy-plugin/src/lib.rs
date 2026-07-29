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
//! recompute_visibility — exclusive system; refresh each player's fog of war from
//!                      owned entities' sight, before acquisition/AI read it
//! recompute_stats    — exclusive system; fold active buffs into effective stats,
//!                      the once-per-tick snapshot consumers read
//! flee               — exclusive system; fleeing-stance entities run from fresh hits
//! auto_engage        — exclusive system; stance-driven target acquisition for idle
//!                      entities
//! tick_orders        — exclusive system; full order lifecycle for alive entities:
//!                        prepare phase: flush cancelled entries, New → InProcessing,
//!                          Suspended → resumed, insert driver components
//!                        watch phase: suspended watchers may interrupt their running
//!                          sub-order (attack-move/guard scanning mid-walk)
//!                        process phase: advance InProcessing front order, remove driver
//!                          components on finish, push chase sub-orders on suspend
//! process_pending_reveals — exclusive system; retry reappearing entities that finished
//!                      an order while boxed-in and still await a free cell
//! process_impacts    — exclusive system; land shots whose flight time has elapsed,
//!                      where the same-tick delivery path lands its damage
//! process_buffs      — exclusive system; age timed buffs (expiries land next tick)
//! process_cooldowns  — exclusive system; age skill cooldowns by one tick
//! process_energy_regen — exclusive system; refill energy pools toward max_energy
//! process_entity_ai  — per-entity AI think (throttled, every N ticks) [not yet implemented]
//! check_game_result  — apply the finish policy; may end the session (last player
//!                      standing, or a scripted scenario's verdict)
//! tick_counter       — advance the simulation tick
//! ```
//!
//! Use `.after(SimulationSet)` to read sim state after the tick completes.

pub mod ai;
mod input;
pub mod map;
pub mod network;
pub mod replay;
pub mod scenario;
mod systems;

pub use ferrets_simulation::spawn;
pub use input::PendingInput;
pub use map::instantiate_map;
pub use network::{
    BlockedStreak, ControlLinks, DesyncTracker, DropConfig, DropIntent, NetworkActive,
    NetworkPlugin, NetworkSession, PauseIntent, PendingPause, Stall, StallInfo, StallVotes,
    detect_drops, install_network_session, net_broadcast, net_checksum, net_control, net_receive,
};
pub use replay::{
    ReplayPlayback, ReplayPlugin, ReplayRecorder, install_replay_playback, install_replay_recorder,
    record_input, supply_replay_input, verify_replay_checksum,
};
pub use scenario::{
    ScenarioObjectives, ScenarioRuntimes, install_scenario_runtime, instantiate_scenario,
    remove_scenario_runtime,
};
pub use systems::flush_input;

use std::sync::Mutex;

use bevy::prelude::*;
use ferrets_simulation::{
    content::registry::ContentRegistry,
    control_groups::ControlGroups,
    entity_index::EntityIndex,
    impacts::PendingImpacts,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    map::Map,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, player_slot::PlayerSlot},
    simulation_id::SimulationIdGenerator,
    visibility::VisibilityGrid,
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
    world.insert_resource(ControlGroups::new(player_count));
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
        let visibility = VisibilityGrid::new(player_count, map.width(), map.height());
        app.insert_resource(session)
            .insert_resource(map)
            .insert_resource(Selection::new(player_count))
            .insert_resource(ControlGroups::new(player_count))
            .insert_resource(visibility)
            .insert_resource(PlayerResources::new(player_count))
            .insert_resource(frames)
            .init_resource::<ContentRegistry>()
            .init_resource::<EntityIndex>()
            .init_resource::<PendingImpacts>()
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
                    // Refresh fog of war before anything acts on it, so this
                    // tick's acquisition and AI see current-tick visibility (dead
                    // entities already removed, no longer granting sight).
                    systems::recompute_visibility,
                    // Fold active buffs into effective stats before consumers
                    // read them, so a buff applied by a command this tick is in
                    // this tick's snapshot.
                    systems::recompute_stats,
                    // Stance-driven initiative first, so a fresh engagement or
                    // flee response executes on the same tick it was decided.
                    systems::flee,
                    systems::auto_engage,
                    systems::tick_orders,
                    // Shots released earlier land here — the same point in the tick
                    // where a hit delivered without a projectile is applied, so both
                    // delivery paths reach their victims on the same schedule.
                    systems::process_impacts,
                    systems::process_pending_reveals,
                    // Age timed buffs; expiries land in the next tick's
                    // recompute_stats snapshot.
                    systems::process_buffs,
                    // Skill cooldowns tick down and energy pools refill.
                    systems::process_cooldowns,
                    systems::process_energy_regen,
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
