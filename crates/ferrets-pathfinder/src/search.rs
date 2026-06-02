//! Grid search utilities: finds the nearest free position around a blocked cell.

use std::collections::{HashSet, VecDeque};

use super::{nav_grid::NavGrid, nav_pos::NavPos};

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
/// Returns `None` if no passable position exists within the grid.
pub fn find_nearest_free_pos(grid: &NavGrid, layer_mask: u32, around: NavPos) -> Option<NavPos> {
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
