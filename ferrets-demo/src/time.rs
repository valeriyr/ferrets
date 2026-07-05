//! Couples render time to the simulation's real tick cadence.
//!
//! All on-screen motion is interpolated against the fixed-tick clock, so it can
//! never outrun the simulation. On top of that, this measures how long each
//! fixed step actually takes to execute and scales virtual time so that if a
//! tick can't be computed within its budget, the whole game slows to match
//! (slow-motion) instead of the renderer racing ahead — the applied speed is
//! `target_tick_time / actual_tick_time`, realized with Bevy's `Time<Virtual>`.

use std::time::Instant;

use bevy::prelude::*;

/// Target wall-time budget for one tick (the fixed timestep is 20 Hz).
const TARGET_TICK_SECS: f32 = 1.0 / 20.0;
/// Lowest virtual-time speed; clamps slow-motion so the game never fully stalls.
const MIN_SPEED: f32 = 0.1;

/// Measures fixed-step execution time and the resulting virtual-time speed.
#[derive(Resource)]
pub struct TickTimer {
    start: Option<Instant>,
    /// Smoothed wall time one tick takes to execute, in seconds.
    pub exec_secs: f32,
    /// The virtual-time speed currently applied (1.0 = real time).
    pub speed: f32,
}

impl Default for TickTimer {
    fn default() -> Self {
        Self {
            start: None,
            exec_secs: TARGET_TICK_SECS,
            speed: 1.0,
        }
    }
}

/// Records the start of a fixed step (run in `FixedFirst`).
pub fn mark_tick_start(mut timer: ResMut<TickTimer>) {
    timer.start = Some(Instant::now());
}

/// Measures the step's execution time and scales virtual time (run in
/// `FixedLast`). Slows the game when a tick costs more than its budget; never
/// speeds it past real time.
pub fn scale_time_to_ticks(mut timer: ResMut<TickTimer>, mut virtual_time: ResMut<Time<Virtual>>) {
    if let Some(start) = timer.start.take() {
        let exec = start.elapsed().as_secs_f32();
        timer.exec_secs = timer.exec_secs * 0.85 + exec * 0.15;
    }

    timer.speed = (TARGET_TICK_SECS / timer.exec_secs.max(1e-6)).clamp(MIN_SPEED, 1.0);
    virtual_time.set_relative_speed(timer.speed);
}
