//! Where a skill's cooldown stands, for an entity's skills and a player's
//! alike.

/// Where a skill's cooldown stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cooldown {
    /// It can be cast.
    Ready,
    /// It cannot be cast for this many more ticks, never zero: the tick that
    /// would leave none turns it ready.
    Cooling {
        /// Ticks left.
        remaining: u32,
    },
}

impl Cooldown {
    /// A cooldown `ticks` long from now: ready at once for a skill declaring
    /// none.
    pub fn of(ticks: u32) -> Self {
        match ticks {
            0 => Cooldown::Ready,
            remaining => Cooldown::Cooling { remaining },
        }
    }

    /// This cooldown one tick later.
    pub fn ticked(self) -> Self {
        match self {
            Cooldown::Ready | Cooldown::Cooling { remaining: 1 } => Cooldown::Ready,
            Cooldown::Cooling { remaining } => Cooldown::Cooling {
                remaining: remaining - 1,
            },
        }
    }
}
