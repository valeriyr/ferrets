//! Patches of map a player sees for a while, whatever stands there.
//!
//! A watch is sight over a patch of map that outlives the cast that made it
//! and then stops. It belongs to no entity, and counts down the ticks it has
//! left: one that reaches zero is dropped as it does, so everything the store
//! holds is in force.

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;

use crate::{session::player_id::PlayerId, simulation_id::SimulationId};

/// One patch of map in a player's sight for a while yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    /// The player whose sight it is; its allies see the patch as they see
    /// anything that player sees.
    pub player: PlayerId,
    /// What cast it, for a reader that wants to name the source of a patch.
    /// It does not decide the watch's life: a watch outlives its caster.
    pub caster: SimulationId,
    /// The cell it is centred on.
    pub center: CellPos,
    /// How far from the centre it reaches, in cells.
    pub radius: u32,
    /// Ticks it still holds for.
    pub remaining: u32,
}

/// Every watch in force, in the order it was cast.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Watches(Vec<Watch>);

impl Watches {
    /// Puts `watch` in force.
    pub fn add(&mut self, watch: Watch) {
        self.0.push(watch);
    }

    /// Takes a tick off every watch and drops any that reached zero.
    pub fn tick_down(&mut self) {
        for watch in &mut self.0 {
            watch.remaining = watch.remaining.saturating_sub(1);
        }
        self.0.retain(|watch| watch.remaining > 0);
    }

    /// The watches in force, in the order they were cast.
    pub fn in_force(&self) -> &[Watch] {
        &self.0
    }
}
