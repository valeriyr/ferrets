//! Grid search utilities: finds free positions around cells and footprints.

use std::collections::{HashSet, VecDeque};

use crate::{layer_mask::LayerMask, nav_grid::NavGrid};
use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};

/// Controls how the BFS expands when it encounters a blocked cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expansion {
    /// BFS expands through any cell, blocked or passable.
    ///
    /// Finds the nearest free position regardless of obstacles between the
    /// start and the result. Use when any nearby free cell is acceptable.
    ThroughBlocked,
    /// BFS only continues expanding from passable cells; blocked cells are dead ends.
    ///
    /// The starting cell is always expanded even if it is blocked.
    /// Use when the result must be reachable without crossing walls.
    ThroughPassable,
}

/// What ground a placement needs under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footing {
    /// Cells nothing else holds: what a footprint claiming its ground needs.
    Free,
    /// Any cell the terrain allows, whoever is standing there: what a footprint
    /// that shares its ground needs.
    Shared,
}

const DIRECTIONS: [(i32, i32); 8] = [
    (0, -1),
    (0, 1),
    (-1, 0),
    (1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// Finds at most `max_amount` placements for a `spawn_size` footprint near the
/// rectangle at `around`, none of them overlapping another, searching outward
/// ring by ring to `max_radius`.
///
/// `footing` says what counts as a placement: ground nobody holds, or ground
/// the terrain merely allows.
///
/// The scan order — rings outward, and within a ring the ground closest to the
/// middle of `around` first, ties by row then column — is what makes the answer
/// the same on every peer. Closest-to-the-middle is what sets what a wide
/// footprint leaves behind about its middle rather than in the corner a scan
/// would otherwise start from.
pub fn find_placements_near(
    grid: &NavGrid,
    layer_mask: impl Into<LayerMask>,
    around: CellRect,
    spawn_size: CellSize,
    max_radius: u32,
    max_amount: usize,
    footing: Footing,
) -> Vec<CellPos> {
    let layer_mask = layer_mask.into();
    let mut found: Vec<CellPos> = Vec::with_capacity(max_amount);

    for radius in 0..=max_radius {
        let min_x = around.origin.x.saturating_sub(radius);
        let min_y = around.origin.y.saturating_sub(radius);
        let max_x = (around.origin.x + around.size.width - 1 + radius).min(grid.width() - 1);
        let max_y = (around.origin.y + around.size.height - 1 + radius).min(grid.height() - 1);

        let mut ring: Vec<CellPos> = Vec::new();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                ring.push(CellPos::new(x, y));
            }
        }
        ring.sort_by_key(|&cell| (from_middle(around, cell, spawn_size), cell.y, cell.x));

        for candidate in ring {
            if found.len() == max_amount {
                return found;
            }
            let fits = match footing {
                Footing::Free => grid.is_footprint_passable_by(layer_mask, candidate, spawn_size),
                Footing::Shared => {
                    grid.is_footprint_terrain_passable_by(layer_mask, candidate, spawn_size)
                }
            };
            if !fits {
                continue;
            }
            let rect = CellRect::new(candidate, spawn_size);
            if found
                .iter()
                .any(|&taken| CellRect::new(taken, spawn_size).intersects(rect))
            {
                continue;
            }
            found.push(candidate);
        }
    }
    found
}

/// Finds the nearest placement for a `spawn_size` footprint near the rectangle
/// at `around`, searching outward ring by ring to `max_radius`.
pub fn find_placement_near(
    grid: &NavGrid,
    layer_mask: impl Into<LayerMask>,
    around: CellRect,
    spawn_size: CellSize,
    max_radius: u32,
    footing: Footing,
) -> Option<CellPos> {
    find_placements_near(grid, layer_mask, around, spawn_size, max_radius, 1, footing)
        .into_iter()
        .next()
}

/// Finds the nearest passable position to `around` using BFS.
///
/// Returns `around` itself if it is already passable.
/// Returns `None` if no passable position exists reachable under `expansion`.
pub fn find_nearest_free_pos(
    grid: &NavGrid,
    layer_mask: impl Into<LayerMask>,
    around: CellPos,
    expansion: Expansion,
) -> Option<CellPos> {
    let layer_mask = layer_mask.into();

    if grid.is_passable_by(layer_mask, around) {
        return Some(around);
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    visited.insert(around);
    queue.push_back(around);

    let w = grid.width() as i32;
    let h = grid.height() as i32;

    while let Some(pos) = queue.pop_front() {
        if expansion == Expansion::ThroughPassable
            && grid.is_occupied_by(layer_mask, pos)
            && pos != around
        {
            continue;
        }

        for &(dx, dy) in &DIRECTIONS {
            let nx = pos.x as i32 + dx;
            let ny = pos.y as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let neighbor = CellPos::new(nx as u32, ny as u32);
            if !visited.insert(neighbor) {
                continue;
            }
            if grid.is_passable_by(layer_mask, neighbor) {
                return Some(neighbor);
            }
            queue.push_back(neighbor);
        }
    }

    None
}

/// How far the middle of a `spawn_size` footprint placed at `candidate` sits
/// from the middle of `around`, as a squared distance in half-cells — whole
/// numbers, so the ordering it gives is the same on every peer.
fn from_middle(around: CellRect, candidate: CellPos, spawn_size: CellSize) -> i64 {
    let middle = |origin: CellPos, size: CellSize| {
        (
            2 * i64::from(origin.x) + i64::from(size.width) - 1,
            2 * i64::from(origin.y) + i64::from(size.height) - 1,
        )
    };
    let (x, y) = middle(around.origin, around.size);
    let (candidate_x, candidate_y) = middle(candidate, spawn_size);
    (candidate_x - x).pow(2) + (candidate_y - y).pow(2)
}
