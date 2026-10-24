//! Stores, for each cell of the game map, its terrain, each layer's relief
//! there, and what occupies it on each layer content defines.

use crate::{
    cliff_crossing::CliffCrossing,
    layer_id::LayerId,
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    relief::Relief,
    slope::{Direction, Slope},
    terrain_id::TerrainId,
    terrain_mask::TerrainMask,
};
use ferrets_geometry::{
    cell_pos::CellPos,
    cell_rect::CellRect,
    cell_size::CellSize,
    projection::{self, Projection},
};

/// Stores per-cell navigation data for the game map.
///
/// Each cell has a **terrain**, shared by every layer, and on each layer a
/// **relief** — a level and a slope — fixed at map build; a layer at one level
/// is flat at it everywhere. Every layer is at every cell. Occupancy lies on
/// two planes per layer: the **occupancy** plane (standing footprints) and the
/// **claim** plane (cells units hold while resting or crossing). Terrain,
/// relief and occupancy together are the **static** plane; the claims are the
/// plane that changes as units move.
#[derive(Debug, Clone)]
pub struct NavGrid {
    width: u32,
    height: u32,
    /// Mask of all registered layers.
    registered: LayerMask,
    /// `terrains[y * width + x]` — what the cell is made of.
    terrains: Vec<TerrainId>,
    /// Each registered layer's relief, indexed by the layer's bit; `None`
    /// for a bit no layer is registered at.
    reliefs: Vec<Option<LayerRelief>>,
    /// `occupancy[y * width + x]` — the set of layers a standing footprint
    /// occupies at that cell.
    occupancy: Vec<LayerMask>,
    /// `claims[y * width + x]` — the set of layers a unit holds at that cell.
    claims: Vec<LayerMask>,
}

/// A layer's relief over the whole grid.
#[derive(Debug, Clone)]
enum LayerRelief {
    /// At one level everywhere, with no slopes.
    Flat(u32),
    /// A level and a slope per cell, `y * width + x`.
    Shaped(Vec<Relief>),
}

impl NavGrid {
    /// Creates a grid with no layers registered, each cell of the terrain
    /// `terrain_at` gives it.
    pub fn new(width: u32, height: u32, terrain_at: impl Fn(CellPos) -> TerrainId) -> Self {
        Self {
            width,
            height,
            registered: LayerMask::EMPTY,
            terrains: CellRect::new(CellPos::new(0, 0), CellSize::new(width, height))
                .cells()
                .map(terrain_at)
                .collect(),
            reliefs: vec![None; u32::BITS as usize],
            occupancy: vec![LayerMask::EMPTY; width as usize * height as usize],
            claims: vec![LayerMask::EMPTY; width as usize * height as usize],
        }
    }

    /// Returns the grid width in cells.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the grid height in cells.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Registers a fully-open layer, flat at `level` everywhere.
    ///
    /// Panics if `layer` is already registered.
    pub fn add_flat_layer(&mut self, layer: impl Into<LayerId>, level: u32) {
        self.register(layer.into(), LayerRelief::Flat(level));
    }

    /// Registers a fully-open layer spanning levels `from` to `to`, each cell
    /// of the relief `relief_at` gives it.
    ///
    /// Panics if `layer` is already registered, or a cell's level lies outside
    /// `from..=to`.
    pub fn add_ranged_layer(
        &mut self,
        layer: impl Into<LayerId>,
        from: u32,
        to: u32,
        relief_at: impl Fn(CellPos) -> Relief,
    ) {
        let reliefs: Vec<Relief> =
            CellRect::new(CellPos::new(0, 0), CellSize::new(self.width, self.height))
                .cells()
                .map(relief_at)
                .collect();
        assert!(
            reliefs
                .iter()
                .all(|relief| (from..=to).contains(&relief.level)),
            "a ranged layer's levels lie within {from}..={to}"
        );
        self.register(layer.into(), LayerRelief::Shaped(reliefs));
    }

    /// Returns a layer's relief at a position, or `None` off the grid.
    ///
    /// Panics if `layer` is not registered.
    pub fn relief(&self, layer: LayerId, pos: CellPos) -> Option<Relief> {
        let relief = self.relief_of(layer);
        self.index(pos).map(|i| match relief {
            LayerRelief::Flat(level) => Relief {
                level: *level,
                slope: None,
            },
            LayerRelief::Shaped(reliefs) => reliefs[i],
        })
    }

    /// Returns a position's terrain, or `None` off the grid.
    pub fn terrain(&self, pos: CellPos) -> Option<TerrainId> {
        self.index(pos).map(|i| self.terrains[i])
    }

    /// Returns a layer's level at a position, or `None` off the grid.
    ///
    /// Panics if `layer` is not registered.
    pub fn cell_level(&self, layer: LayerId, pos: CellPos) -> Option<u32> {
        self.relief(layer, pos).map(|relief| relief.level)
    }

    /// Returns the slope at a position on a layer, or `None` on flat ground or
    /// off the grid.
    ///
    /// Panics if `layer` is not registered.
    pub fn slope(&self, layer: LayerId, pos: CellPos) -> Option<Slope> {
        self.relief(layer, pos).and_then(|relief| relief.slope)
    }

    /// Returns `true` if a mover of `shape` may cross the edge between two
    /// neighboring positions, `from` and `to` one cell apart along one axis,
    /// on the layer it moves on: on the same level; one level apart where the
    /// lower is a ramp rising across the edge; one level apart anywhere for
    /// [`CliffCrossing::Cross`]; always for a mover with no crossing, whose
    /// layer is at one level. Whether a cliff cell may be stood on at all is
    /// [`cell_fits`](Self::cell_fits)'s question.
    ///
    /// Panics if the two are not neighbors along one axis. Out-of-bounds
    /// positions always return `false`.
    pub fn can_cross(&self, from: CellPos, to: CellPos, shape: MoverShape) -> bool {
        let edge = Direction::of_step(to.x as i32 - from.x as i32, to.y as i32 - from.y as i32);
        assert!(
            edge.is_cardinal(),
            "an edge lies between cardinal neighbors"
        );
        let (Some(from_relief), Some(to_relief)) =
            (self.relief(shape.layer, from), self.relief(shape.layer, to))
        else {
            return false;
        };
        match shape.cliff_crossing {
            None => true,
            Some(CliffCrossing::Ramp | CliffCrossing::Cross)
                if from_relief.level == to_relief.level =>
            {
                true
            }
            Some(CliffCrossing::Ramp | CliffCrossing::Cross)
                if from_relief.level.abs_diff(to_relief.level) != 1 =>
            {
                false
            }
            Some(CliffCrossing::Cross) => true,
            Some(CliffCrossing::Ramp) if from_relief.level < to_relief.level => {
                rises_across(from_relief.slope, edge)
            }
            Some(CliffCrossing::Ramp) => rises_across(to_relief.slope, edge.opposite()),
        }
    }

    /// Returns `true` if a footprint of `shape` can step from the anchor
    /// `from` to the neighboring anchor `to`, as far as levels go: every cell
    /// it enters is crossable from the cell one step back. A diagonal step
    /// must be crossable by both cardinal two-step paths around it.
    ///
    /// Panics if the two anchors are not neighbors.
    pub fn can_step(&self, from: CellPos, to: CellPos, shape: MoverShape) -> bool {
        let (dx, dy) = (to.x as i32 - from.x as i32, to.y as i32 - from.y as i32);
        assert!(
            dx.abs() <= 1 && dy.abs() <= 1 && (dx, dy) != (0, 0),
            "a step goes to a neighboring anchor"
        );
        match (shape.cliff_crossing, dx, dy) {
            (None, _, _) => true,
            (Some(CliffCrossing::Ramp | CliffCrossing::Cross), 0, _)
            | (Some(CliffCrossing::Ramp | CliffCrossing::Cross), _, 0) => {
                self.can_step_along(to, dx, dy, shape)
            }
            (Some(CliffCrossing::Ramp | CliffCrossing::Cross), _, _) => {
                let across = CellPos::new(to.x, from.y);
                let down = CellPos::new(from.x, to.y);
                self.can_step(from, across, shape)
                    && self.can_step(across, to, shape)
                    && self.can_step(from, down, shape)
                    && self.can_step(down, to, shape)
            }
        }
    }

    /// Returns `true` if `to` is joined to `from` for a mover of `shape`:
    /// every edge of both cardinal paths between them — along x then y, and
    /// along y then x — is crossable. A position joins itself.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn joins(&self, from: CellPos, to: CellPos, shape: MoverShape) -> bool {
        self.index(from).is_some()
            && self.index(to).is_some()
            && self.path_crosses(from, CellPos::new(to.x, from.y), shape)
            && self.path_crosses(CellPos::new(to.x, from.y), to, shape)
            && self.path_crosses(from, CellPos::new(from.x, to.y), shape)
            && self.path_crosses(CellPos::new(from.x, to.y), to, shape)
    }

    /// Returns `true` if every two neighboring cells of the shape's footprint
    /// at `anchor` are crossable for its cliff crossing.
    pub fn holds_together(&self, anchor: CellPos, shape: MoverShape) -> bool {
        match shape.cliff_crossing {
            None => true,
            Some(CliffCrossing::Ramp | CliffCrossing::Cross) => {
                CellRect::new(anchor, shape.size).cells().all(|cell| {
                    let east = CellPos::new(cell.x + 1, cell.y);
                    let south = CellPos::new(cell.x, cell.y + 1);
                    (east.x >= anchor.x + shape.size.width || self.can_cross(cell, east, shape))
                        && (south.y >= anchor.y + shape.size.height
                            || self.can_cross(cell, south, shape))
                })
            }
        }
    }

    /// Returns `true` if the footprint `from` of a body of `shape` reaches
    /// `goal` within `range` under `projection`; a reach of one cell or less —
    /// a touch — also needs some cell of `from` joined, for `shape`, to a cell
    /// of `goal` it shares or neighbors.
    pub fn reaches(
        &self,
        projection: Projection,
        from: CellRect,
        shape: MoverShape,
        goal: CellRect,
        range: u32,
    ) -> bool {
        projection.in_range_for_rects(from, goal, range) && self.touches(from, shape, goal, range)
    }

    /// Returns `true` if a mover of `shape` anchored at `anchor` stops within
    /// `stop` of `goal` under the same touch rule as
    /// [`reaches`](Self::reaches).
    pub fn accepts(
        &self,
        projection: Projection,
        anchor: CellPos,
        shape: MoverShape,
        goal: CellRect,
        stop: u32,
    ) -> bool {
        projection.in_reach(anchor, shape.size, goal, stop)
            && self.touches(CellRect::new(anchor, shape.size), shape, goal, stop)
    }

    /// Returns `true` if every cell of the shape's footprint at `anchor` is on
    /// one level of the layer it stands on, and none is part of a slope there.
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn is_flat(&self, anchor: CellPos, shape: MoverShape) -> bool {
        let Some(anchor_relief) = self.relief(shape.layer, anchor) else {
            return false;
        };
        CellRect::new(anchor, shape.size).cells().all(|cell| {
            self.relief(shape.layer, cell)
                .is_some_and(|relief| relief.level == anchor_relief.level && relief.slope.is_none())
        })
    }

    /// Sets whether a standing footprint occupies a position on all layers
    /// matched by `mask`; terrain is untouched either way.
    ///
    /// Out-of-bounds positions are silently ignored.
    pub fn set_occupied_by(&mut self, mask: impl Into<LayerMask>, pos: CellPos, occupied: bool) {
        let mask = mask.into();

        self.assert_registered(mask);

        let Some(i) = self.index(pos) else { return };

        if occupied {
            self.occupancy[i] |= mask;
        } else {
            self.occupancy[i] &= !mask;
        }
    }

    /// Sets whether a unit holds the position on all layers matched by
    /// `mask`.
    ///
    /// Out-of-bounds positions are silently ignored.
    pub fn set_claimed_by(&mut self, mask: impl Into<LayerMask>, pos: CellPos, claimed: bool) {
        let mask = mask.into();

        self.assert_registered(mask);

        let Some(i) = self.index(pos) else { return };

        if claimed {
            self.claims[i] |= mask;
        } else {
            self.claims[i] &= !mask;
        }
    }

    /// Returns `true` if the position is occupied — by a standing footprint or
    /// a unit's claim — on **any** layer in `mask`, whatever its terrain.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| (self.occupancy[i] | self.claims[i]) & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Clears every unit claim on every layer, leaving terrain and standing
    /// occupancy untouched — for rebuilding the claim plane from current positions.
    pub fn clear_claims(&mut self) {
        self.claims.fill(LayerMask::EMPTY);
    }

    /// Returns `true` if a unit holds the position on **any** layer in
    /// `mask`, ignoring standing footprints.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_claimed_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.claims[i] & mask != LayerMask::EMPTY)
            .unwrap_or(false)
    }

    /// Returns `true` if a standing footprint occupies the position on
    /// **any** layer in `mask`, ignoring unit claims and terrain.
    ///
    /// Out-of-bounds positions always return `true`.
    pub fn is_statically_occupied_by(&self, mask: impl Into<LayerMask>, pos: CellPos) -> bool {
        let mask = mask.into();

        self.assert_registered(mask);

        self.index(pos)
            .map(|i| self.occupancy[i] & mask != LayerMask::EMPTY)
            .unwrap_or(true)
    }

    /// Returns `true` if the position's terrain is one of `terrains`, whatever
    /// stands or rests on it.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn is_on_terrains(&self, terrains: TerrainMask, pos: CellPos) -> bool {
        self.terrain(pos)
            .is_some_and(|terrain| terrains.meets(terrain.into()))
    }

    /// Returns `true` if a cell of `shape` may stand at the position: on one
    /// of its terrains, not on a cliff of its layer unless it crosses cliffs,
    /// and free of standing occupancy and unit claims alike on its layers.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn cell_fits(&self, pos: CellPos, shape: MoverShape) -> bool {
        self.stands_on_ground(pos, shape) && !self.is_occupied_by(shape.layers, pos)
    }

    /// Returns `true` if a cell of `shape` may stand at the position as
    /// [`cell_fits`](Self::cell_fits) says, whatever units claim.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn cell_fits_statically(&self, pos: CellPos, shape: MoverShape) -> bool {
        self.stands_on_ground(pos, shape) && !self.is_statically_occupied_by(shape.layers, pos)
    }

    /// Returns `true` if a cell of `shape` may stand on the ground at `pos`,
    /// whatever occupies it: on one of its terrains, and not on a cliff of its
    /// layer unless it crosses cliffs.
    ///
    /// Out-of-bounds positions always return `false`.
    pub fn stands_on_ground(&self, pos: CellPos, shape: MoverShape) -> bool {
        let (Some(terrain), Some(relief)) = (self.terrain(pos), self.relief(shape.layer, pos))
        else {
            return false;
        };
        let slope_allows = match (relief.slope, shape.cliff_crossing) {
            (Some(Slope::Cliff(_)), Some(CliffCrossing::Ramp)) => false,
            (Some(Slope::Cliff(_)), Some(CliffCrossing::Cross) | None)
            | (Some(Slope::Ramp(_)) | None, _) => true,
        };
        shape.terrains.meets(terrain.into()) && slope_allows
    }

    /// Returns `true` if every cell of the shape's footprint at `anchor`
    /// [`cell_fits`](Self::cell_fits) — ground it may stand on, free on its
    /// layers — and the footprint [holds together](Self::holds_together).
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn fits(&self, anchor: CellPos, shape: MoverShape) -> bool {
        CellRect::new(anchor, shape.size)
            .cells()
            .all(|cell| self.cell_fits(cell, shape))
            && self.holds_together(anchor, shape)
    }

    /// Returns `true` if every cell of the shape's footprint at `anchor`
    /// [`cell_fits_statically`](Self::cell_fits_statically) — ground it may
    /// stand on, free of static occupants on its layers — and the footprint
    /// [holds together](Self::holds_together).
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn fits_statically(&self, anchor: CellPos, shape: MoverShape) -> bool {
        CellRect::new(anchor, shape.size)
            .cells()
            .all(|cell| self.cell_fits_statically(cell, shape))
            && self.holds_together(anchor, shape)
    }

    /// Returns `true` if `shape` anchored at `anchor` may stand on the ground
    /// of every cell its footprint covers, whoever else is there.
    ///
    /// Footprints reaching out of bounds always return `false`.
    pub fn fits_ground(&self, anchor: CellPos, shape: MoverShape) -> bool {
        CellRect::new(anchor, shape.size)
            .cells()
            .all(|cell| self.stands_on_ground(cell, shape))
    }

    /// Panics in debug builds if `mask` contains any unregistered layer bits.
    #[inline]
    fn assert_registered(&self, mask: LayerMask) {
        debug_assert!(
            mask & !self.registered == LayerMask::EMPTY,
            "mask contains unregistered layers"
        );
    }

    /// Whether a reach of `range` from the footprint `from` of a body of
    /// `shape` to `goal` meets the touch rule: beyond one cell always; within
    /// it, some cell of `from` is joined, for `shape`, to a cell of `goal` it
    /// shares or neighbors.
    fn touches(&self, from: CellRect, shape: MoverShape, goal: CellRect, range: u32) -> bool {
        match range {
            0 | 1 => from.cells().any(|cell| {
                goal.cells().any(|reached| {
                    projection::chebyshev(cell, reached) <= 1 && self.joins(cell, reached, shape)
                })
            }),
            _ => true,
        }
    }

    /// Whether a footprint of `shape` stepping by `(dx, dy)`, one cell along
    /// one axis, onto the anchor `to` crosses only edges its cliff crossing allows:
    /// every cell it lands on from the cell one step back.
    fn can_step_along(&self, to: CellPos, dx: i32, dy: i32, shape: MoverShape) -> bool {
        CellRect::new(to, shape.size).cells().all(|cell| {
            let back = CellPos::new((cell.x as i32 - dx) as u32, (cell.y as i32 - dy) as u32);
            self.can_cross(back, cell, shape)
        })
    }

    /// Whether every edge along the straight cardinal run from `from` to `to`,
    /// which share a row or a column, is crossable.
    fn path_crosses(&self, from: CellPos, to: CellPos, shape: MoverShape) -> bool {
        let (dx, dy) = (
            (to.x as i32 - from.x as i32).signum(),
            (to.y as i32 - from.y as i32).signum(),
        );
        let mut at = from;
        while at != to {
            let next = CellPos::new((at.x as i32 + dx) as u32, (at.y as i32 + dy) as u32);
            if !self.can_cross(at, next, shape) {
                return false;
            }
            at = next;
        }
        true
    }

    /// Registers `layer` with its relief.
    ///
    /// Panics if `layer` is already registered.
    fn register(&mut self, layer: LayerId, relief: LayerRelief) {
        assert_eq!(
            self.registered & layer,
            LayerMask::EMPTY,
            "layer {layer} is already registered"
        );
        self.registered |= layer;
        self.reliefs[layer.trailing_zeros() as usize] = Some(relief);
    }

    /// The relief of a registered layer.
    ///
    /// Panics if `layer` is not registered.
    fn relief_of(&self, layer: LayerId) -> &LayerRelief {
        self.reliefs[layer.trailing_zeros() as usize]
            .as_ref()
            .unwrap_or_else(|| panic!("layer {layer} is not registered"))
    }

    /// Converts a grid position to a flat array index, or `None` if out of bounds.
    fn index(&self, pos: CellPos) -> Option<usize> {
        if pos.x >= self.width || pos.y >= self.height {
            return None;
        }
        Some(pos.y as usize * self.width as usize + pos.x as usize)
    }
}

/// Whether `slope` is part of a ramp rising across the cardinal `edge`.
fn rises_across(slope: Option<Slope>, edge: Direction) -> bool {
    match slope {
        Some(Slope::Ramp(rises)) => rises.crosses(edge),
        Some(Slope::Cliff(_)) | None => false,
    }
}
