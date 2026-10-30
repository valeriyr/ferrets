//! Straight lines across the cell grid between cell centers.

use crate::{cell_pos::CellPos, cell_rect::CellRect};

/// One thing a line across the grid passes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crossing {
    /// The interior of a cell.
    Cell(CellPos),
    /// The corner four cells share, named by the cell it is the lowest
    /// corner of.
    Corner(CellPos),
}

/// Everything the segment from the center of `from` to the center of `to`
/// passes through, in order from `from`: each cell whose interior it
/// crosses, and each cell corner it passes exactly through but its own two
/// ends. A stretch running along a cell edge crosses no interior.
pub fn crossings(from: CellRect, to: CellRect) -> Crossings {
    // Points are counted in half cells, where every cell corner and center
    // falls on a whole number.
    let (from_x, from_y) = center(from);
    let (to_x, to_y) = center(to);
    let (delta_x, delta_y) = (to_x - from_x, to_y - from_y);
    // Distances along the segment are counted in `span` steps from `from`
    // (0) to `to` (`span`), so that every grid line it crosses falls on a
    // whole step.
    let span = delta_x.abs().max(1) * delta_y.abs().max(1);
    Crossings {
        from: (from_x, from_y),
        delta: (delta_x, delta_y),
        span,
        lines: [
            Lines::new(from_x, delta_x, span),
            Lines::new(from_y, delta_y, span),
        ],
        reached: 0,
        corner: None,
    }
}

/// The crossings of one segment, walked from its start as they are asked
/// for.
#[derive(Debug, Clone)]
pub struct Crossings {
    /// The segment's start, in half cells.
    from: (i64, i64),
    /// The segment's run along each axis, in half cells.
    delta: (i64, i64),
    /// The steps the segment's length is counted in.
    span: i64,
    /// The grid lines it crosses next along each axis.
    lines: [Lines; 2],
    /// The step walked to so far.
    reached: i64,
    /// A corner passed at `reached`, given after the cell before it.
    corner: Option<Crossing>,
}

impl Iterator for Crossings {
    type Item = Crossing;

    fn next(&mut self) -> Option<Crossing> {
        loop {
            if let Some(corner) = self.corner.take() {
                return Some(corner);
            }
            if self.reached == self.span {
                return None;
            }
            let from = self.reached;
            let stop = self
                .lines
                .iter()
                .map(|lines| lines.next)
                .min()
                .expect("a segment has two axes")
                .min(self.span);
            for lines in &mut self.lines {
                if lines.next == stop {
                    lines.next += lines.step;
                }
            }
            self.reached = stop;
            if stop < self.span {
                self.corner = self.corner_at(stop);
            }
            if let Some(cell) = self.cell_between(from, stop) {
                return Some(cell);
            }
        }
    }
}

impl Crossings {
    /// The cell the stretch between two steps lies in, or `None` for a
    /// stretch along a grid line.
    fn cell_between(&self, from: i64, to: i64) -> Option<Crossing> {
        // The stretch's midpoint, scaled by 2 × span.
        let mid_x = self.from.0 * 2 * self.span + self.delta.0 * (from + to);
        let mid_y = self.from.1 * 2 * self.span + self.delta.1 * (from + to);
        let cell = 4 * self.span;
        (mid_x.rem_euclid(cell) != 0 && mid_y.rem_euclid(cell) != 0).then(|| {
            Crossing::Cell(CellPos::new(
                u32::try_from(mid_x.div_euclid(cell)).unwrap(),
                u32::try_from(mid_y.div_euclid(cell)).unwrap(),
            ))
        })
    }

    /// The cell corner the segment passes at `step`, if it passes one.
    fn corner_at(&self, step: i64) -> Option<Crossing> {
        // The point, scaled by span.
        let at_x = self.from.0 * self.span + self.delta.0 * step;
        let at_y = self.from.1 * self.span + self.delta.1 * step;
        let corner = 2 * self.span;
        (at_x.rem_euclid(corner) == 0 && at_y.rem_euclid(corner) == 0).then(|| {
            Crossing::Corner(CellPos::new(
                u32::try_from(at_x / corner).unwrap(),
                u32::try_from(at_y / corner).unwrap(),
            ))
        })
    }
}

/// The steps at which a segment crosses the grid lines of one axis.
#[derive(Debug, Clone, Copy)]
struct Lines {
    /// The step of the next line it crosses; the segment's span once none is
    /// left.
    next: i64,
    /// The steps between one line and the next.
    step: i64,
}

impl Lines {
    /// The lines crossed by a segment starting at `start` and running
    /// `delta` half cells, its length counted in `span` steps.
    fn new(start: i64, delta: i64, span: i64) -> Self {
        if delta == 0 {
            return Self {
                next: span,
                step: 0,
            };
        }
        // Lines lie on even half cells: one half cell away from an odd
        // start, two from an even one, whichever way the segment runs.
        let scale = span / delta.abs();
        let first = if start.rem_euclid(2) == 0 { 2 } else { 1 };
        Self {
            next: (first * scale).min(span),
            step: 2 * scale,
        }
    }
}

/// The center of `rect`, in half cells.
fn center(rect: CellRect) -> (i64, i64) {
    (
        2 * i64::from(rect.origin.x) + i64::from(rect.size.width),
        2 * i64::from(rect.origin.y) + i64::from(rect.size.height),
    )
}
