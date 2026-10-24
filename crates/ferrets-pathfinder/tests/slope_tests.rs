//! Slopes: the eight directions a step or a slope may take.

use ferrets_pathfinder::slope::Direction;

//
// ─── Directions ───────────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "a step (0, 0) goes to no neighbor")]
fn step_that_stays_put_has_no_direction() {
    Direction::of_step(0, 0);
}
