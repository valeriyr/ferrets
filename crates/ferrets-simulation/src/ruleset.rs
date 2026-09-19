//! The rules one game is played under.
//!
//! What belongs here is behaviour the simulation follows regardless of which
//! entity types are on the map — the rules of *this* game rather than facts
//! about what things are, which is content's business. A game states every one
//! of them: nothing stands behind a rule left unsaid.
//!
//! They travel with the game they belong to — a
//! [`Skirmish`](crate::skirmish::Skirmish) spells them out, a
//! [`Scenario`](crate::scenario::Scenario) carries its own — and are handed to
//! the [`GameSession`](crate::session::GameSession) that game configures,
//! which is where the simulation reads them.

use serde::{Deserialize, Serialize};

/// How many remains the map holds at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemainsLimit {
    /// As many as fall; decay is what thins them.
    Unbounded,
    /// At most this many. A death beyond the limit leaves no body: what lies
    /// there already stays, and decay is what makes room again.
    AtMost(u32),
}

/// The rules one game is played under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruleset {
    /// How many remains the map holds at once.
    remains: RemainsLimit,
}

impl Ruleset {
    /// Creates a new `Ruleset` with the given data.
    ///
    /// Panics if the remains limit is zero: a map that holds no remains is one
    /// whose types declare none.
    pub fn new(remains: RemainsLimit) -> Self {
        assert!(
            remains != RemainsLimit::AtMost(0),
            "a remains limit of none is declared by leaving remains undeclared"
        );

        Self { remains }
    }

    /// How many remains the map holds at once.
    #[inline]
    pub fn remains(self) -> RemainsLimit {
        self.remains
    }
}
