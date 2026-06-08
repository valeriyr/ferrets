//! Grid search utilities: finds the nearest free position around a blocked cell.

use std::collections::{HashSet, VecDeque};

use crate::layer_mask::LayerMask;

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
