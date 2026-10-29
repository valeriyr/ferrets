//! Progress of an in-flight form change.

use bevy_ecs::prelude::*;
use ferrets_content::{entity_type_def::EntityTypeId, pool_def::PoolId};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedU64;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::components::patience::Patience;

/// Ground held ahead of a form change landing on it.
#[derive(Debug, Clone)]
pub struct MorphReservation {
    /// Exactly the cells this change claimed — cells someone else already
    /// held stay theirs — so releasing them never takes anything that was
    /// not this change's to take. Not a rectangle: cells of the destination
    /// footprint the entity's own standing claim covers are left under that
    /// claim, and must survive a cancel.
    pub cells: Vec<CellPos>,
    /// The layers the cells are claimed on — the ones the destination occupies.
    pub layers: LayerMask,
}

/// A change of form under way.
#[derive(Debug, Clone)]
pub struct MorphUnderWay {
    /// Ticks spent so far.
    pub progress: u32,
    /// The ground claimed ahead for the destination footprint. `None` when
    /// nothing is held ahead: the transition does not reserve, or its
    /// destination claims no cells.
    pub reservation: Option<MorphReservation>,
    /// What each pool held when the change started.
    pub before: Vec<(PoolId, FixedU64)>,
}

/// How far a change of form has come.
#[derive(Debug, Clone)]
pub enum MorphStage {
    /// Not begun: nothing paid or reserved, the entity in its own form,
    /// waiting on bodies making way off the ground the change needs.
    Clearing(Patience),
    /// Begun: paid, the destination's supply held, the interim form worn.
    UnderWay(MorphUnderWay),
}

/// A Morph order's change of form on this entity.
#[derive(Component, Debug, Clone)]
pub struct MorphComponent {
    /// The type the entity was when the order started, which declares the
    /// transition: the terms are read from it, and a change that ends early
    /// returns to it.
    pub from: EntityTypeId,
    /// The name of the type being changed into.
    pub into: String,
    /// How far the change has come.
    pub stage: MorphStage,
}

impl MorphComponent {
    /// A change of `from` into `into` not begun, that has not waited yet.
    pub fn clearing(from: EntityTypeId, into: String) -> Self {
        Self {
            from,
            into,
            stage: MorphStage::Clearing(Patience::starting()),
        }
    }

    /// A change of `from` into `into` begun this tick, its pools holding
    /// `before`, no ground reserved yet.
    pub fn under_way(from: EntityTypeId, into: String, before: Vec<(PoolId, FixedU64)>) -> Self {
        Self {
            from,
            into,
            stage: MorphStage::UnderWay(MorphUnderWay {
                progress: 0,
                reservation: None,
                before,
            }),
        }
    }

    /// Whether the change has begun.
    pub fn is_under_way(&self) -> bool {
        self.as_under_way().is_some()
    }

    /// The change under way, or `None` while it has not begun.
    pub fn as_under_way(&self) -> Option<&MorphUnderWay> {
        match &self.stage {
            MorphStage::Clearing(_) => None,
            MorphStage::UnderWay(under_way) => Some(under_way),
        }
    }
}
