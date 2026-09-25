//! Patches of map a player sees for a while, whatever stands there.
//!
//! A watch is sight over a patch of map that outlives the cast that made it
//! and then stops. It belongs to no entity, and counts down the ticks it has
//! left: one that reaches zero is dropped as it does, so everything the store
//! holds is in force.

use bevy_ecs::prelude::*;
use ferrets_content::detection::Detection;
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection};
use ferrets_pathfinder::layer_mask::LayerMask;

use crate::{
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
};

/// One patch of map in a player's sight for a while yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    /// The player whose sight it is; its allies see the patch as they see
    /// anything that player sees.
    pub player: PlayerId,
    /// What cast it, for a reader that wants to name the source of a patch.
    /// It does not decide the watch's life: a watch outlives its caster.
    pub caster: SimulationId,
    /// The cell it is centered on.
    pub center: CellPos,
    /// How far from the center it reaches, in cells.
    pub radius: u32,
    /// Ticks it still holds for.
    pub remaining: u32,
    /// What it does for the player against the concealed entities standing on
    /// the patch.
    pub detection: Detection,
}

impl Watch {
    /// Every cell of the patch, row-major, unclipped to any map.
    pub fn cells(&self) -> Vec<CellPos> {
        projection::circle_cells(self.patch(), self.radius)
    }

    /// Whether the patch holds `cell`.
    fn holds(&self, cell: CellPos) -> bool {
        projection::in_circle(cell, self.patch(), self.radius)
    }

    /// The footprint the radius reaches out from: the center cell alone.
    fn patch(&self) -> CellRect {
        CellRect::new(self.center, CellSize::ONE)
    }
}

/// Every watch in force, in the order it was cast.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Watches(Vec<Watch>);

impl Watches {
    /// Puts `watch` in force.
    fn add(&mut self, watch: Watch) {
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

/// Whether a watch in force for `player` or an ally holds `cell` and detects on
/// one of `layers`.
pub fn detects(world: &World, player: PlayerId, cell: CellPos, layers: LayerMask) -> bool {
    let session = world.resource::<GameSession>();
    world.resource::<Watches>().in_force().iter().any(|watch| {
        session.are_allied(player, watch.player)
            && match watch.detection {
                Detection::Blind => false,
                Detection::Reveals(revealed) => revealed & layers != LayerMask::EMPTY,
            }
            && watch.holds(cell)
    })
}

/// Puts a watch in force for `player`, `radius` cells around `center`,
/// holding for `duration` ticks and detecting as `detection` says, cast by
/// `caster`.
pub fn open(
    world: &mut World,
    player: PlayerId,
    caster: SimulationId,
    center: CellPos,
    radius: u32,
    duration: u32,
    detection: Detection,
) {
    // The tick of the cast ages the watch once before the next recompute
    // reads it, so the seat is one above the duration: the patch holds
    // through the `duration` ticks after the one it was cast in.
    world.resource_mut::<Watches>().add(Watch {
        player,
        caster,
        center,
        radius,
        remaining: duration + 1,
        detection,
    });
}
