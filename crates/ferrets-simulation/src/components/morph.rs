//! Progress of an in-flight form change.

use bevy_ecs::prelude::*;
use ferrets_geometry::cell_pos::CellPos;
use ferrets_pathfinder::layer_mask::LayerMask;

/// Ground held ahead of a form change landing on it.
#[derive(Debug, Clone)]
pub struct MorphReservation {
    /// Exactly the cells this change claimed — cells someone else already
    /// held stay theirs — so releasing them never takes anything that was
    /// not this change's to take. Not a rectangle: cells of the destination
    /// footprint the entity's own standing claim covers are left under that
    /// claim, and must survive a cancel.
    pub cells: Vec<CellPos>,
    /// The layers the cells are claimed on — the destination's occupation.
    pub mask: LayerMask,
}

/// A type change under way on this entity.
///
/// The destination is held by name, because transitions are inherently
/// circular — two forms can each name the other — and no registration order
/// can resolve both ends to a handle. Requirements and admission lists are
/// name-based for the same reason.
#[derive(Component, Debug, Clone)]
pub struct MorphComponent {
    /// The type being changed into.
    pub type_name: String,
    /// Ticks spent so far.
    pub progress: u32,
    /// The ground claimed ahead for the destination footprint. `None` when
    /// the transition revalidates at completion instead of reserving.
    pub reservation: Option<MorphReservation>,
}
