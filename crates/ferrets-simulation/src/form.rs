//! One entity type's body on a map: what it declares, the cells it spans and
//! the plane it marks.

use ferrets_content::{
    entity_type_def::EntityTypeDef,
    location::{LocationDef, Solidity},
};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_math::FixedU64;
use ferrets_nav_grid::layer_mask::LayerMask;

use crate::{
    movement_model::MovementModel,
    occupancy::{GridMark, OccupancyClass},
    simulation_id::SimulationId,
};

/// One entity type's body on a map of one movement model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    /// Where and how the type stands, as its content declares it.
    location: LocationDef,
    /// The cells the body spans on each axis: on a continuous map a mover's
    /// circle's square, `max(1, ⌈2 × radius⌉)` cells a side; every other
    /// body's declared footprint.
    extent: CellSize,
    /// The plane the body marks its cells on.
    occupancy: OccupancyClass,
}

impl Form {
    /// `def`'s body on a map of `model`.
    ///
    /// Panics if `def` declares no location, or is a mover on a continuous
    /// map with no radius.
    pub fn of(def: &EntityTypeDef, model: MovementModel) -> Self {
        Self::checked(def, model).expect("a type read for its form stands somewhere")
    }

    /// `def`'s body on a map of `model`, or `None` for a type that stands
    /// nowhere.
    ///
    /// Panics if `def` is a mover on a continuous map with no radius.
    pub fn checked(def: &EntityTypeDef, model: MovementModel) -> Option<Self> {
        let location = def.location?;
        let extent = match (model, def.can_move()) {
            (MovementModel::Continuous, true) => square_of(
                location
                    .radius()
                    .expect("a continuous map's movers declare a radius"),
            ),
            (MovementModel::Cell, true)
            | (MovementModel::Cell | MovementModel::Continuous, false) => location.size(),
        };
        Some(Self {
            location,
            extent,
            occupancy: OccupancyClass::of(def),
        })
    }

    /// Where and how the type stands, as its content declares it.
    #[inline]
    pub fn location_def(self) -> LocationDef {
        self.location
    }

    /// The layers the body occupies.
    #[inline]
    pub fn occupies(self) -> LayerMask {
        self.location.occupies()
    }

    /// How the body stands toward others over its cells.
    #[inline]
    pub fn solidity(self) -> Solidity {
        self.location.solidity()
    }

    /// The cells the body spans on each axis.
    #[inline]
    pub fn extent(self) -> CellSize {
        self.extent
    }

    /// The plane the body marks its cells on.
    #[inline]
    pub fn occupancy(self) -> OccupancyClass {
        self.occupancy
    }

    /// The cells the body covers anchored at `anchor`.
    #[inline]
    pub fn rect_at(self, anchor: CellPos) -> CellRect {
        CellRect::new(anchor, self.extent)
    }

    /// What `holder`'s body anchored at `anchor` marks on the grid, or `None`
    /// for one whose solidity marks no cells.
    pub fn mark_at(self, holder: SimulationId, anchor: CellPos) -> Option<GridMark> {
        self.solidity().claims_cells().then(|| GridMark {
            holder,
            footprint: self.rect_at(anchor),
            layers: self.occupies(),
            class: self.occupancy,
        })
    }
}

/// The square of whole cells a circle of `radius` spans: `max(1, ⌈2r⌉)` a
/// side.
fn square_of(radius: FixedU64) -> CellSize {
    let side = (radius * 2).ceil().to_num::<u32>().max(1);
    CellSize::new(side, side)
}
