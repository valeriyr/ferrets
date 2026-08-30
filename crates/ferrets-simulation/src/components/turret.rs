//! Runtime state for the turrets an entity carries.

use bevy_ecs::prelude::*;
use ferrets_math::facing::Facing;

use crate::order::AttackTarget;

/// One turret's state: where it points, what it is working, and how far into the
/// swing it is.
///
/// The bearing outlives any one fight — a gun left trained where it last fought
/// is where it starts the next one — which is why it is kept here rather than
/// with whatever fight is in progress.
#[derive(Debug, Clone, Copy)]
pub struct TurretState {
    /// Where the gun points.
    pub bearing: Facing,
    /// What it is working, if anything — what its body was ordered onto, or what
    /// it found for itself.
    pub quarry: Option<AttackTarget>,
    /// Ticks into the current attack cycle.
    pub phase: u32,
}

impl TurretState {
    /// A gun just mounted, pointing the way its body does and working nothing.
    pub fn mounted(bearing: Facing) -> Self {
        Self {
            bearing,
            quarry: None,
            phase: 0,
        }
    }

    /// Puts the gun onto `quarry`, starting its cycle over whenever that is
    /// something other than what it was working: a swing committed against one
    /// body does not land on the next. `None` takes it off everything.
    pub fn switch_quarry(&mut self, quarry: Option<AttackTarget>) {
        if self.quarry != quarry {
            self.phase = 0;
        }
        self.quarry = quarry;
    }
}

/// The turrets an entity carries, in the order its type mounts them.
#[derive(Component, Debug, Default)]
pub struct TurretsComponent(pub Vec<TurretState>);
