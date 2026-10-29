//! How long something has waited on what it needs, against a limit.

/// What something waiting does this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// It waits on.
    Waits,
    /// It has waited as long as it waits, and gives up.
    GivesUp,
}

/// How long something has waited on what it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patience {
    /// The ticks it has waited.
    waited: u32,
}

impl Patience {
    /// Something that has not waited yet.
    pub fn starting() -> Self {
        Self { waited: 0 }
    }

    /// The ticks it has waited.
    pub fn waited(&self) -> u32 {
        self.waited
    }

    /// Counts a tick of waiting: [`Waiting::GivesUp`] once it has waited more
    /// than `limit` ticks, [`Waiting::Waits`] until then, and always with no
    /// limit.
    pub fn wait(&mut self, limit: Option<u32>) -> Waiting {
        self.waited = self.waited.saturating_add(1);
        match limit {
            Some(limit) if self.waited > limit => Waiting::GivesUp,
            Some(_) | None => Waiting::Waits,
        }
    }
}
