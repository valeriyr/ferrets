//! Grid search utilities: finds free positions around cells and footprints.

use std::collections::{HashSet, VecDeque};

use ferrets_geometry::{cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize};
use ferrets_nav_grid::{mover_shape::MoverShape, nav_grid::NavGrid};

/// Controls how the BFS expands when it encounters a blocked cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expansion {
    /// BFS expands through any cell, blocked or passable.
    ///
    /// Finds the nearest free position regardless of obstacles between the
    /// start and the result. Use when any nearby free cell is acceptable.
    ThroughBlocked,
    /// BFS only continues expanding from passable cells, across edges a step
    /// may cross; blocked cells are dead ends.
    ///
    /// The starting cell is always expanded even if it is blocked.
    /// Use when the result must be reachable without crossing walls.
    ThroughPassable,
}

/// What ground a placement needs under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footing {
    /// Ground its shape may stand on that nothing else holds: what a footprint
    /// claiming its ground needs.
    Free,
    /// Any ground its shape may stand on, whoever is standing there: what a
    /// footprint that shares its ground needs.
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

/// Finds at most `max_amount` placements for `shape` near the rectangle at
/// `around`, none of them overlapping another, searching outward ring by ring
/// to `max_radius`.
///
/// `footing` says what counts as a placement: ground nobody holds, or ground
/// its shape may stand on whoever holds it; `accept` takes or refuses each
/// anchor that footing admits.
///
/// The scan order — rings outward, and within a ring the ground closest to the
/// middle of `around` first, ties by row then column — is what makes the answer
/// the same on every peer. Closest-to-the-middle is what sets what a wide
/// footprint leaves behind about its middle rather than in the corner a scan
/// would otherwise start from.
pub fn find_placements_near(
    grid: &NavGrid,
    shape: MoverShape,
    around: CellRect,
    max_radius: u32,
    max_amount: usize,
    footing: Footing,
    mut accept: impl FnMut(CellPos) -> bool,
) -> Vec<CellPos> {
    let mut found: Vec<CellPos> = Vec::with_capacity(max_amount);
    if max_amount == 0 {
        return found;
    }
    walk_rings(
        grid,
        around,
        max_radius,
        |cell| (from_middle(around, cell, shape.size), cell.y, cell.x),
        |candidate| {
            let rect = CellRect::new(candidate, shape.size);
            if fits(grid, shape, candidate, footing)
                && accept(candidate)
                && !found
                    .iter()
                    .any(|&taken| CellRect::new(taken, shape.size).intersects(rect))
            {
                found.push(candidate);
            }
            if found.len() == max_amount {
                Walk::Stop
            } else {
                Walk::Next
            }
        },
    );
    found
}

/// Finds the nearest placement for `shape` near the rectangle at `around`
/// that `footing` admits and `accept` takes, searching outward ring by ring
/// to `max_radius`; within a ring the ground closest to `toward` comes first,
/// then the order [`find_placements_near`] keeps.
pub fn find_placement_near_toward(
    grid: &NavGrid,
    shape: MoverShape,
    around: CellRect,
    max_radius: u32,
    footing: Footing,
    toward: CellRect,
    mut accept: impl FnMut(CellPos) -> bool,
) -> Option<CellPos> {
    find_placement_near_toward_where(grid, around, shape.size, max_radius, toward, |cell| {
        fits(grid, shape, cell, footing) && accept(cell)
    })
}

/// Finds the first anchor near the rectangle at `around` that `accept` takes
/// for a `spawn_size` footprint, in the order [`find_placement_near_toward`]
/// tries them: ring by ring outward to `max_radius`, each cell once, within a
/// ring the ground closest to `toward` first.
pub fn find_placement_near_toward_where(
    grid: &NavGrid,
    around: CellRect,
    spawn_size: CellSize,
    max_radius: u32,
    toward: CellRect,
    mut accept: impl FnMut(CellPos) -> bool,
) -> Option<CellPos> {
    let mut found = None;
    walk_rings(
        grid,
        around,
        max_radius,
        |cell| {
            (
                from_middle(toward, cell, spawn_size),
                from_middle(around, cell, spawn_size),
                cell.y,
                cell.x,
            )
        },
        |cell| {
            if accept(cell) {
                found = Some(cell);
                Walk::Stop
            } else {
                Walk::Next
            }
        },
    );
    found
}

/// Finds the nearest placement for `shape` near the rectangle at `around`
/// that `footing` admits and `accept` takes, searching outward ring by ring
/// to `max_radius`.
pub fn find_placement_near(
    grid: &NavGrid,
    shape: MoverShape,
    around: CellRect,
    max_radius: u32,
    footing: Footing,
    accept: impl FnMut(CellPos) -> bool,
) -> Option<CellPos> {
    find_placements_near(grid, shape, around, max_radius, 1, footing, accept)
        .into_iter()
        .next()
}

/// Finds the nearest cell to `around` a single-cell `shape` fits on, using BFS.
///
/// Returns `around` itself if the shape already fits there.
/// Returns `None` if no such cell exists reachable under `expansion`.
pub fn find_nearest_free_pos(
    grid: &NavGrid,
    shape: MoverShape,
    around: CellPos,
    expansion: Expansion,
) -> Option<CellPos> {
    if grid.fits(around, shape) {
        return Some(around);
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    visited.insert(around);
    queue.push_back(around);

    let w = grid.width() as i32;
    let h = grid.height() as i32;

    while let Some(pos) = queue.pop_front() {
        let expands = match expansion {
            Expansion::ThroughPassable => pos == around || grid.fits(pos, shape),
            Expansion::ThroughBlocked => true,
        };
        if !expands {
            continue;
        }

        for &(dx, dy) in &DIRECTIONS {
            let nx = pos.x as i32 + dx;
            let ny = pos.y as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let neighbor = CellPos::new(nx as u32, ny as u32);
            let steppable = match expansion {
                Expansion::ThroughPassable => grid.can_step(pos, neighbor, shape),
                Expansion::ThroughBlocked => true,
            };
            if !steppable {
                continue;
            }
            if !visited.insert(neighbor) {
                continue;
            }
            if grid.fits(neighbor, shape) {
                return Some(neighbor);
            }
            queue.push_back(neighbor);
        }
    }

    None
}

/// Whether a ring walk goes on to the next cell.
enum Walk {
    /// On to the next cell.
    Next,
    /// Done.
    Stop,
}

/// Whether `shape` anchored at `candidate` has the ground `footing` asks for.
fn fits(grid: &NavGrid, shape: MoverShape, candidate: CellPos, footing: Footing) -> bool {
    match footing {
        Footing::Free => grid.fits(candidate, shape),
        Footing::Shared => grid.fits_ground(candidate, shape),
    }
}

/// Hands `visit` every anchor near the rectangle at `around`, ring by ring
/// outward to `max_radius`, each cell once and within a ring in `order`, until
/// it says stop.
fn walk_rings<K: Ord>(
    grid: &NavGrid,
    around: CellRect,
    max_radius: u32,
    order: impl Fn(CellPos) -> K,
    mut visit: impl FnMut(CellPos) -> Walk,
) {
    // The rectangle the rings so far have covered.
    let mut covered: Option<CellRect> = None;
    for radius in 0..=max_radius {
        let min_x = around.origin.x.saturating_sub(radius);
        let min_y = around.origin.y.saturating_sub(radius);
        let max_x = (around.origin.x + around.size.width - 1 + radius).min(grid.width() - 1);
        let max_y = (around.origin.y + around.size.height - 1 + radius).min(grid.height() - 1);

        let mut ring: Vec<CellPos> = Vec::new();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let cell = CellPos::new(x, y);
                if !covered.is_some_and(|covered| covered.contains(cell)) {
                    ring.push(cell);
                }
            }
        }
        ring.sort_by_key(|&cell| order(cell));
        for cell in ring {
            match visit(cell) {
                Walk::Next => {}
                Walk::Stop => return,
            }
        }
        covered = Some(CellRect::new(
            CellPos::new(min_x, min_y),
            CellSize::new(max_x - min_x + 1, max_y - min_y + 1),
        ));
    }
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
