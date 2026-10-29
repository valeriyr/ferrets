//! Which ground a body reaches from where it comes: the regions of its shape
//! a mover walks into and the levels a body that never walks steps onto.

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_nav_grid::{layer_id::LayerId, mover_shape::MoverShape, nav_grid::NavGrid};

use crate::{landing::ComesFrom, map::Map};

/// Where a footprint a body comes out of stands against the body's layer.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Relation {
    /// On the body's own layer.
    Same,
    /// On a layer above it.
    Above,
    /// On a layer not above it.
    Below,
}

/// The regions of `shape` a mover coming `from` there walks into: at a
/// point, the region of the point's cell, or, where its shape cannot stand on
/// that cell, the regions it steps into from there; out of a footprint on its
/// own layer or a layer below, the regions within the footprint's reach — a
/// touch across an edge it could step for a reach of one, by distance alone
/// beyond — at the levels it steps onto across the footprint's edge; coming
/// down from a layer above, the regions within that reach at any level.
pub(crate) fn walked_into(map: &Map, shape: MoverShape, from: ComesFrom) -> Vec<u32> {
    let grid = map.nav_grid();
    let hierarchy = map.hierarchy();
    let regions_of = |anchors: &mut dyn Iterator<Item = CellPos>| {
        let mut regions: Vec<u32> = Vec::new();
        for anchor in anchors {
            if let Some(region) = hierarchy.region_of(anchor, shape)
                && !regions.contains(&region)
            {
                regions.push(region);
            }
        }
        regions
    };
    match from {
        ComesFrom::Point(cell) => match hierarchy.region_of(cell, shape) {
            Some(region) => vec![region],
            // A point on ground the body refuses is left the way a footprint
            // is.
            None => regions_of(&mut stepped_into(
                map,
                shape,
                CellRect::new(cell, CellSize::ONE),
                1,
                Relation::Same,
            )),
        },
        ComesFrom::Footprint { rect, layer, reach } => {
            match relation(grid, layer, shape.layer, rect) {
                Relation::Above => {
                    regions_of(&mut stepped_into(map, shape, rect, reach, Relation::Above))
                }
                // Beyond a reach of one, distance carries the body over what
                // lies between, but only onto the levels it steps onto across
                // the edge.
                relation @ (Relation::Same | Relation::Below) => {
                    let levels = levels_beside(map, shape, rect, relation);
                    regions_of(&mut stepped_into(map, shape, rect, reach, relation).filter(
                        |&anchor| {
                            let level =
                                highest_level(grid, shape.layer, CellRect::new(anchor, shape.size))
                                    .expect("an anchor within reach stands on the map");
                            levels.contains(&level)
                        },
                    ))
                }
            }
        }
    }
}

/// Whether a body of `shape` anchored at `cell` stands in one of `regions`.
pub(crate) fn holds(map: &Map, shape: MoverShape, regions: &[u32], cell: CellPos) -> bool {
    map.hierarchy()
        .region_of(cell, shape)
        .is_some_and(|region| regions.contains(&region))
}

/// The levels a body of `shape` steps onto across `rect`'s edge, out of a
/// footprint standing as `relation` says against its layer.
pub(crate) fn levels_beside(
    map: &Map,
    shape: MoverShape,
    rect: CellRect,
    relation: Relation,
) -> Vec<u32> {
    let mut levels: Vec<u32> = Vec::new();
    for anchor in stepped_into(map, shape, rect, 1, relation) {
        let level = highest_level(
            map.nav_grid(),
            shape.layer,
            CellRect::new(anchor, shape.size),
        )
        .expect("an anchor within reach stands on the map");
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    levels
}

/// Where a footprint on `source` stands against `layer` over `rect`: above
/// when the source's highest level there lies over the layer's.
///
/// Panics if `rect` runs off the map.
pub(crate) fn relation(
    grid: &NavGrid,
    source: LayerId,
    layer: LayerId,
    rect: CellRect,
) -> Relation {
    if source == layer {
        return Relation::Same;
    }
    let level = |layer| {
        highest_level(grid, layer, rect).expect("a footprint a body comes out of stands on the map")
    };
    if level(source) > level(layer) {
        Relation::Above
    } else {
        Relation::Below
    }
}

/// The highest level of `layer` under `rect`, or `None` off the map.
pub(crate) fn highest_level(grid: &NavGrid, layer: LayerId, rect: CellRect) -> Option<u32> {
    rect.cells().try_fold(None, |highest: Option<u32>, cell| {
        let level = grid.relief(layer, cell)?.level;
        Some(Some(highest.map_or(level, |highest| highest.max(level))))
    })?
}

/// Every anchor of a `size` footprint within `reach` cells of `rect` by its
/// bounding square, the footprint on the map: none for one wider than it.
pub(crate) fn anchors_within(
    grid: &NavGrid,
    rect: CellRect,
    size: CellSize,
    reach: u32,
) -> impl Iterator<Item = CellPos> {
    let min_x = rect
        .origin
        .x
        .saturating_sub((size.width - 1).saturating_add(reach));
    let min_y = rect
        .origin
        .y
        .saturating_sub((size.height - 1).saturating_add(reach));
    // A footprint wider than the map has no anchor on it.
    let window = match (
        grid.width().checked_sub(size.width),
        grid.height().checked_sub(size.height),
    ) {
        (Some(last_x), Some(last_y)) => Some((
            min_y
                ..=(rect.origin.y + rect.size.height - 1)
                    .saturating_add(reach)
                    .min(last_y),
            min_x
                ..=(rect.origin.x + rect.size.width - 1)
                    .saturating_add(reach)
                    .min(last_x),
        )),
        (None, _) | (_, None) => None,
    };
    window.into_iter().flat_map(|(rows, columns)| {
        rows.flat_map(move |y| columns.clone().map(move |x| CellPos::new(x, y)))
    })
}

/// The anchors within `reach` of `rect` a body of `shape` steps into from
/// there: within a reach of one only across an edge it could step, as any
/// touch is judged; any, coming down from a layer above.
fn stepped_into(
    map: &Map,
    shape: MoverShape,
    rect: CellRect,
    reach: u32,
    relation: Relation,
) -> impl Iterator<Item = CellPos> + '_ {
    let grid = map.nav_grid();
    let projection = map.projection();
    anchors_within(grid, rect, shape.size, reach).filter(move |&anchor| {
        let footprint = CellRect::new(anchor, shape.size);
        match relation {
            Relation::Above => projection.in_range_for_rects(rect, footprint, reach),
            Relation::Same | Relation::Below => {
                grid.reaches(projection, rect, shape, footprint, reach)
            }
        }
    })
}
