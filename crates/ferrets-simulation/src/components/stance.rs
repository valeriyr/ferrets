//! Stance state for simulation entities.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// How an entity responds to enemies on its own initiative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stance {
    /// Runs from whatever damages it; never fights back.
    Flee,
    /// Never engages or flees on its own initiative.
    HoldFire,
    /// Engages what enters weapon range; never moves to fight.
    StandGround,
    /// Engages what enters acquisition range, chases as long as the target
    /// stays near the spot the fight started at, and returns when it ends.
    Defend,
}

impl Stance {
    /// The stance's name in the scripting vocabulary.
    pub fn name(self) -> &'static str {
        match self {
            Stance::Flee => "flee",
            Stance::HoldFire => "hold_fire",
            Stance::StandGround => "stand_ground",
            Stance::Defend => "defend",
        }
    }

    /// Whether the stance picks targets on its own initiative.
    pub fn auto_engages(self) -> bool {
        match self {
            Stance::StandGround | Stance::Defend => true,
            Stance::Flee | Stance::HoldFire => false,
        }
    }

    /// Whether the stance runs from whatever damages it.
    pub fn flees(self) -> bool {
        matches!(self, Stance::Flee)
    }
}

/// The entity's current stance.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StanceComponent(pub Stance);
