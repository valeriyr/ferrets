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
//! All simulation systems run in `FixedUpdate` inside [`SimulationSet`], whose
//! phases ([`FixedUpdateSet`]) give the tick loop its shape — a plugin joins a
//! phase rather than ordering against another plugin's systems, and the sequence
//! is stated once where the phases are configured. `FixedLast` closes the step
//! in the same way ([`FixedLastSet`]).
//!
//! The order mirrors a classic RTS tick loop:
//!
//! ```text
//! ── Receive ──────────────────────────────────────────────────────────────────
//! net_receive        — remote players' frames land; peers' checksums recorded
//! net_control        — tick-aligned pause/speed/drop decisions apply
//! ── Sources ──────────────────────────────────────────────────────────────────
//! supply_replay_input — the recording, the sole frame source during playback
//! supply_unmanned_input — idle frames for locally-sourced slots with no brain
//! supply_ai_input    — scripted AI frame source (thinks on its cadence)
//! ── Commit ───────────────────────────────────────────────────────────────────
//! flush_input        — drain PendingInput into InputFrames (runs while session is active)
//! ── Decide ───────────────────────────────────────────────────────────────────
//! detect_drops       — a stall past its grace window becomes a drop
//! ── Broadcast ────────────────────────────────────────────────────────────────
//! net_broadcast      — (re)broadcast the frame window
//! net_checksum       — exchange this tick's state hash
//! ── Execute ──────────────────────────────────────────────────────────────────
//! command_executor   — translate InputFrames → OrderQueueComponent mutations
//! ── Simulate ─────────────────────────────────────────────────────────────────
//! [ApplyDeferred]
//! process_dying      — exclusive system; advance Die orders, despawn entities that
//!                      finished dying
//! recompute_visibility — exclusive system; refresh each player's fog of war from
//!                      owned entities' sight, before acquisition/AI read it
//! recompute_entity_stats — exclusive system; fold each entity's buffs and its owner's
//!                      player-level buffs and modifiers into effective stats, the
//!                      once-per-tick snapshot consumers read
//! recompute_player_stats — exclusive system; refold player stats from applied
//!                      modifiers and what the player's active buffs grant
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
//! process_impacts    — exclusive system; land shots whose flight time has elapsed,
//!                      where the same-tick delivery path lands its damage
//! process_pending_reveals — exclusive system; retry reappearing entities that finished
//!                      an order while boxed-in and still await a free cell
//! process_entity_buffs — exclusive system; age entities' timed buffs (expiries
//!                      land next tick)
//! process_player_buffs — exclusive system; age players' timed buffs likewise
//! process_entity_skills — exclusive system; age entity-skill cooldowns by one tick
//! process_player_skills — exclusive system; age player-skill cooldowns
//! process_energy_regen — exclusive system; refill energy pools toward max_energy
//! process_health_regen — exclusive system; refill health pools toward max_health,
//!                      skipping the dying and the still-under-construction
//! process_entity_ai  — per-entity AI think (throttled, every N ticks) [not yet implemented]
//! check_game_result  — apply the finish policy; may end the session (last player
//!                      standing, or a scripted scenario's verdict)
//! tick_counter       — advance the simulation tick
//! ```
//!
//! Use `.after(SimulationSet)` to read sim state after the tick completes; the
//! phases are part of that set, so it covers the whole tick.
//!
//! # Cadence
//!
//! How fast those ticks come is decided outside that set: each frame the fixed
//! timestep is derived from the session's speed and from what a tick was
//! measured to cost (see [`tick`]). None of it is visible to the simulation.

pub mod ai;
mod input;
pub mod intents;
pub mod map;
pub mod network;
pub mod replay;
pub mod scenario;
mod simulation;
mod systems;
pub mod tick;

pub use ferrets_simulation::spawn;
pub use input::PendingInput;
pub use intents::{
    pause::{PauseIntent, apply_local_pause},
    speed::{SpeedIntent, apply_local_speed},
};
pub use map::instantiate_map;
pub use network::{
    BlockedStreak, ControlLinks, DesyncTracker, DropConfig, DropIntent, FrameMargins,
    NetworkActive, NetworkPlugin, NetworkSession, PeerCapacities, PendingPause, PendingSpeed,
    Stall, StallInfo, StallVotes, detect_drops, install_network_session, net_broadcast,
    net_checksum, net_control, net_receive,
};
pub use replay::{
    ReplayPlugin,
    playback::{
        PlaybackReport, ReplayPlayback, run_playback, supply_replay_input, verify_replay_checksum,
    },
    recorder::{ReplayRecorder, record_input},
};
pub use scenario::{
    ScenarioObjectives, ScenarioRuntimes, install_scenario_runtime, instantiate_scenario,
    remove_scenario_runtime,
};
pub use systems::flush_input;
pub use tick::{
    MARGIN_HEADROOM, MAX_FACTOR, MIN_NOMINAL_SHARE, MIN_PEER_THROTTLE, ManualTick, NO_THROTTLE,
    NominalTimestep, Seek, Step, TARGET_LOAD, ThrottleConfig, TickPacing, apply_seek, apply_step,
    mark_tick_start, measure_tick, run_tick, run_tick_while_paused, run_until_tick,
    sustainable_factor, sync_fixed_timestep, throttle_for,
};

use std::sync::Mutex;

use bevy::prelude::*;
use ferrets_content::registry::ContentRegistry;
use ferrets_simulation::{map::Map, session::GameSession};

/// System set containing all simulation systems.
///
/// Schedule systems that read sim state `.after(SimulationSet)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimulationSet;

/// The phases of `FixedUpdate`, in the order they run — the tick loop's shape,
/// stated once.
///
/// Every plugin places its systems in a phase instead of ordering against
/// another plugin's systems by name, so the sequence is readable in one place
/// and a new plugin has somewhere to belong. The order is total: nothing in the
/// tick is left to the executor's choice, because `push_frame` is
/// first-write-wins — two sources reaching the same slot in an undeclared order
/// would resolve differently on different nodes, which is a desync.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FixedUpdateSet {
    /// Remote frames land and the session-level decisions riding with them
    /// apply (a tick-aligned pause, a speed change, an authoritative drop).
    Receive,
    /// The frame sources fill the slots nobody has filled: a recording during
    /// playback, idle frames for unmanned slots, the scripted AI. Ordered after
    /// [`Receive`](Self::Receive) so a synthesized frame can never beat the real
    /// one a peer sent.
    ///
    /// The recording and the live sources are not ordered against each other,
    /// and need not be: they are mutually exclusive by run condition — the
    /// recording drives every slot when a playback is installed, the live
    /// sources only when none is — so no tick ever runs both.
    Sources,
    /// The local player's pending input is committed to the queue.
    Commit,
    /// What the committed queue is missing becomes a decision: a stall, and the
    /// drop that may follow it.
    Decide,
    /// The frame window and the state checksum go out to the peers.
    Broadcast,
    /// Committed commands become orders.
    Execute,
    /// The simulation advances, and the tick counter with it.
    Simulate,
}

/// The phases of `FixedLast`, in the order they run: what a completed tick owes,
/// then closing the measurement of what it cost.
///
/// Declared as sets rather than left to system-to-system ordering so a plugin
/// adding end-of-tick work states which phase it belongs to, instead of naming
/// another plugin's systems — and so the order stays stated in one place as more
/// of them appear.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FixedLastSet {
    /// Work that belongs to the tick just executed: recording its input,
    /// verifying its checksum, anything a game does with a completed tick. Its
    /// cost is part of what the tick cost.
    Work,
    /// Closes the step's cost measurement, so everything in [`Work`](Self::Work)
    /// counts toward what a tick is measured to cost.
    Measure,
}

fn session_is_active(session: Res<GameSession>) -> bool {
    session.is_active()
}

fn session_is_not_paused(session: Res<GameSession>) -> bool {
    !session.is_paused()
}

fn session_is_advancing(session: Res<GameSession>) -> bool {
    session.is_advancing()
}

/// (Re)installs the per-game state: sizes the per-player stores to the session's
/// slots and clears everything a previous game in the same app may have left
/// behind. Call at game start once [`GameSession`] holds the finalized
/// configuration (e.g. from a lobby), since the plugin is built before the real
/// slots are known.
///
/// Everything the plugin holds *for one game* is cleared here — the simulation's
/// transient state, the control plane's pending decisions and observations, and
/// what the cadence measured — so a game need not know which of the engine's
/// resources are per-game and which last the app's lifetime. Left alone are the
/// game's own choices ([`DropConfig`], [`ThrottleConfig`]) and the cadence it
/// installed ([`NominalTimestep`]).
///
/// The map-shaped resources — [`Map`] and [`VisibilityGrid`] — are not installed
/// here: the scene spawner builds them from the game's map data (see
/// [`instantiate_map`]).
pub fn install_game_resources(world: &mut World) {
    // Each subsystem owns its own per-game roster; this only gathers them so
    // every entry path installs the same set.
    simulation::install_per_game(world);
    intents::pause::install_per_game(world);
    intents::speed::install_per_game(world);
    network::install_per_game(world);
    tick::install_per_game(world);
}

/// Tears the per-game state down when leaving a game: despawns every simulation
/// entity, resets the simulation stores to their pre-game state, and removes
/// what the game's entry path installed — the network session, a recorder or
/// playback, any AI or scenario runtimes, and pending step/seek requests. The
/// mirror of [`install_game_resources`], owned here for the same reason: a game
/// need not know which engine resources a finished game leaves behind, and one
/// left behind would keep acting — a live network session keeps receiving, an
/// installed playback keeps supplying.
///
/// The game's own state — its scene, UI, and the [`Map`] its next game installs —
/// stays the game's to clean up.
pub fn teardown_game_resources(world: &mut World) {
    // The mirror of installing: each subsystem removes its own.
    simulation::remove_per_game(world);
    intents::pause::remove_per_game(world);
    intents::speed::remove_per_game(world);
    network::remove_per_game(world);
    ai::remove_ai_runtimes(world);
    scenario::remove_scenario_runtime(world);
    replay::recorder::remove_per_game(world);
    replay::playback::remove_per_game(world);
    tick::remove_per_game(world);
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

        app.insert_resource(session);
        // The map-shaped pair and the per-game rosters come from the functions
        // that own them, so the plugin cannot drift from what the game-start
        // installers build.
        map::install_per_game(app.world_mut(), map);
        simulation::install_per_game(app.world_mut());
        intents::pause::install_per_game(app.world_mut());
        intents::speed::install_per_game(app.world_mut());
        tick::install_per_game(app.world_mut());
        app.init_resource::<ContentRegistry>()
            .init_resource::<tick::NominalTimestep>()
            .init_resource::<tick::ThrottleConfig>()
            // The requested speed and the throttle become a cadence here, before
            // the fixed loop decides how many ticks this frame owes; the two
            // measuring systems bracket each step.
            .add_systems(First, tick::sync_fixed_timestep)
            .add_systems(FixedFirst, tick::mark_tick_start)
            // The tick's phases, ordered once here — every plugin joins one
            // instead of ordering against another plugin's systems by name. They
            // are also part of `SimulationSet`, so a game's `.after(SimulationSet)`
            // still covers the whole tick.
            .configure_sets(
                FixedUpdate,
                (
                    FixedUpdateSet::Receive,
                    FixedUpdateSet::Sources,
                    FixedUpdateSet::Commit,
                    FixedUpdateSet::Decide,
                    FixedUpdateSet::Broadcast,
                    FixedUpdateSet::Execute,
                    FixedUpdateSet::Simulate,
                )
                    .chain()
                    .in_set(SimulationSet),
            )
            // The step's closing phases, ordered once here; `measure_tick` sits
            // in the last one so it observes everything the tick caused.
            .configure_sets(
                FixedLast,
                (FixedLastSet::Work, FixedLastSet::Measure).chain(),
            )
            .add_systems(FixedLast, tick::measure_tick.in_set(FixedLastSet::Measure))
            // A requested step or seek runs its ticks itself, so it belongs
            // outside the fixed loop — and ahead of it. Applied in `Update` it
            // would land in the middle of the frame: the fixed loop and whatever
            // the game already drew would have seen the tick it started from,
            // and the interpolation snapshot taken during that loop would be
            // blended against positions far later. Here the jump is over before
            // the frame does anything with it. Pause and speed intents apply
            // first, so a pause pressed together with a step lands before it.
            .add_systems(
                PreUpdate,
                (
                    // Gated on an active session as well as on there being no
                    // control plane: between games the session is the inert
                    // pending placeholder, and steering that would carry into
                    // the next game (`configure` replaces the slots, not the
                    // pause or the speed).
                    intents::pause::apply_local_pause.run_if(
                        session_is_active.and(not(resource_exists::<network::NetworkActive>)),
                    ),
                    intents::speed::apply_local_speed.run_if(
                        session_is_active.and(not(resource_exists::<network::NetworkActive>)),
                    ),
                    tick::apply_step.run_if(resource_exists::<tick::Step>),
                    tick::apply_seek.run_if(resource_exists::<tick::Seek>),
                )
                    .chain(),
            )
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
                    flush_input
                        .in_set(FixedUpdateSet::Commit)
                        .run_if(not(resource_exists::<ReplayPlayback>)),
                    // Not while an installed recording has nothing for this
                    // tick: the executor is what re-derives running-or-blocked
                    // from the queue, so letting it run past the recording's end
                    // would un-block the session on the warmup frames the engine
                    // seeded and replay ticks that were never recorded.
                    systems::command_executor
                        .in_set(FixedUpdateSet::Execute)
                        .run_if(not(replay::playback::replay_exhausted)),
                )
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
                    systems::recompute_entity_stats,
                    systems::recompute_player_stats,
                    // Stance-driven initiative first, so a fresh engagement or
                    // flee response executes on the same tick it was decided.
                    systems::flee,
                    systems::auto_engage,
                    // The hierarchy's one mutation point: fold in footprint
                    // changes before orders path against it, never lazily at
                    // query time.
                    systems::refresh_nav_hierarchy,
                    systems::tick_orders,
                    // Garrisoned passengers fight outside the order lifecycle,
                    // right after it: their shots join this tick's impacts on
                    // the same schedule as ordered attacks.
                    systems::process_garrison_attacks,
                    // Continuous-model contact: bodies moved by their orders
                    // this tick push each other apart before anything reads
                    // the settled positions.
                    systems::resolve_pushing,
                    // Shots released earlier land here — the same point in the tick
                    // where a hit delivered without a projectile is applied, so both
                    // delivery paths reach their victims on the same schedule.
                    systems::process_impacts,
                    systems::process_pending_reveals,
                    // Age timed buffs; expiries land in the next tick's
                    // recompute snapshots.
                    systems::process_entity_buffs,
                    systems::process_player_buffs,
                    // Skill cooldowns tick down and the per-entity pools refill,
                    // after every source of damage and spending this tick has been
                    // applied.
                    systems::process_entity_skills,
                    systems::process_player_skills,
                    (systems::process_energy_regen, systems::process_health_regen).chain(),
                    systems::check_game_result,
                    systems::tick_counter,
                )
                    .chain()
                    .in_set(FixedUpdateSet::Simulate)
                    .run_if(session_is_advancing),
            );
    }
}
