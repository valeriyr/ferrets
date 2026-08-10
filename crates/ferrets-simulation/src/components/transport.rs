//! Passenger-carrying runtime state for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{components::chase::ChaseState, simulation_id::SimulationId};

/// A transporter's live hold state: who is aboard and how boarding is paced.
#[derive(Component, Debug, Default)]
pub struct TransporterComponent {
    /// The passengers currently aboard.
    pub passengers: BTreeSet<SimulationId>,
    /// The earliest tick the next boarding may complete at. Boarders arriving
    /// sooner wait in place.
    pub boarding_ready_at: u32,
}

/// Marks a passenger riding inside a transporter.
#[derive(Component, Debug, Clone, Copy)]
pub struct BoardedComponent {
    /// The holder this passenger rides in.
    pub holder: SimulationId,
}

/// A garrisoned attacker's firing state, present while it rides a holder that
/// lets passengers fight.
#[derive(Component, Debug, Default)]
pub struct GarrisonFireComponent {
    /// Ticks into the current attack cycle.
    pub phase: u32,
    /// The target the passenger is working on, if it has one.
    pub target: Option<SimulationId>,
}

/// Per-entity in-flight boarding state.
#[derive(Component, Debug)]
pub struct BoardComponent {
    /// The transporter being boarded.
    pub target: SimulationId,
    /// The last chase round toward the transporter; identical rounds
    /// accumulate until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}

impl BoardComponent {
    /// Creates in-flight boarding state aimed at `target`.
    pub fn new(target: SimulationId) -> Self {
        Self {
            target,
            last_chase: None,
        }
    }
}

/// Per-entity in-flight fetching state.
#[derive(Component, Debug)]
pub struct LoadComponent {
    /// The entity being fetched aboard.
    pub target: SimulationId,
    /// The last chase round toward the fetched entity; identical rounds
    /// accumulate until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}

impl LoadComponent {
    /// Creates in-flight fetching state aimed at `target`.
    pub fn new(target: SimulationId) -> Self {
        Self {
            target,
            last_chase: None,
        }
    }
}

/// Per-entity in-flight unloading state.
#[derive(Component, Debug)]
pub struct UnloadComponent {
    /// Where to let the passengers out, if the order named a destination.
    pub at: Option<FixedUVec2>,
    /// Ticks left before the next passenger may step out.
    pub cooldown: u32,
    /// The last chase round toward the destination; identical rounds
    /// accumulate until the chase gives up (see [`ChaseState`]).
    pub last_chase: ChaseState,
}

impl UnloadComponent {
    /// Creates in-flight unloading state aimed at `at`.
    pub fn new(at: Option<FixedUVec2>) -> Self {
        Self {
            at,
            cooldown: 0,
            last_chase: None,
        }
    }
}
