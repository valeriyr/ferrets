//! The rules a game is played under: what they refuse to be.

use ferrets_simulation::ruleset::{RemainsLimit, Ruleset};

#[test]
#[should_panic(expected = "a remains limit of none is declared by leaving remains undeclared")]
fn limit_of_none_panics() {
    Ruleset::new(RemainsLimit::AtMost(0));
}
