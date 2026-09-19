//! The rules every demo game is played under.

use ferrets_simulation::ruleset::{RemainsLimit, Ruleset};

/// The rules the demo's games run under, stated where a game is seated rather
/// than in the content: a campaign mission may want a field of its own.
///
/// Two hundred bodies is well past any ordinary fight, and low enough that
/// killing in bulk in the sandbox shows the oldest making way.
pub fn demo() -> Ruleset {
    Ruleset::new(RemainsLimit::AtMost(200))
}
