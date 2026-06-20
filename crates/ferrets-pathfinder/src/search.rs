//! Grid search utilities: finds free positions around cells and footprints.

use std::collections::{HashSet, VecDeque};

use crate::{layer_mask::LayerMask, nav_size::NavSize};

use super::{nav_grid::NavGrid, nav_pos::NavPos};

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

/// Finds a free position for a footprint of `spawn_size` on `layer_mask`,
/// scanning outward from the rectangle of cells at `around`/`around_size` up to
/// `max_radius` rings away.
///
/// Cells are scanned ring by ring in row-major order, so the result is
/// deterministic. Returns `None` when nothing is free within the search radius.
pub fn find_placement_near(
    grid: &NavGrid,
    layer_mask: impl Into<LayerMask>,
    around: NavPos,
    around_size: NavSize,
    spawn_size: NavSize,
    max_radius: u32,
) -> Option<NavPos> {
    let layer_mask = layer_mask.into();

    for radius in 0..=max_radius {
        let min_x = around.x.saturating_sub(radius);
        let min_y = around.y.saturating_sub(radius);
        let max_x = (around.x + around_size.width - 1 + radius).min(grid.width() - 1);
        let max_y = (around.y + around_size.height - 1 + radius).min(grid.height() - 1);

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let candidate = NavPos::new(x, y);
                if grid.is_footprint_passable_by(layer_mask, candidate, spawn_size) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Finds the nearest passable position to `around` using BFS.
///
/// Returns `around` itself if it is already passable.
/// Returns `None` if no passable position exists reachable under `expansion`.
pub fn find_nearest_free_pos(
    grid: &NavGrid,
    layer_mask: impl Into<LayerMask>,
    around: NavPos,
    expansion: Expansion,
) -> Option<NavPos> {
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
            let neighbor = NavPos::new(nx as u32, ny as u32);
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
