//! Content-defined transport capability: whom an entity carries, and on what
//! terms.

use crate::{affiliation::Affiliation, kinds::Kinds};

/// What happens to the passengers when their holder dies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassengerFate {
    /// Passengers die with the holder.
    Destroy,
    /// Passengers are placed around the holder's footprint; one that cannot be
    /// placed dies anyway.
    Eject,
}

/// What passengers do while aboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassengerConduct {
    /// Passengers sit the ride out and do nothing.
    Shelter,
    /// Armed passengers fire their own weapons from inside.
    Fight,
}

/// Content-defined transport capability: the passengers an entity admits and
/// the terms it holds them on. How much fits aboard is the `cargo_capacity`
/// stat, so the modifier pipeline can move it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransporterDef {
    /// Whom it admits.
    carries: Kinds,
    /// Whose units may board.
    boarding: Affiliation,
    /// What happens to the passengers when the holder dies.
    passenger_fate: PassengerFate,
    /// What passengers do while aboard.
    conduct: PassengerConduct,
}

impl TransporterDef {
    /// Creates a new `TransporterDef` with the given data.
    pub fn new(
        carries: Kinds,
        boarding: Affiliation,
        passenger_fate: PassengerFate,
        conduct: PassengerConduct,
    ) -> Self {
        Self {
            carries,
            boarding,
            passenger_fate,
            conduct,
        }
    }

    /// Whom it admits.
    #[inline]
    pub fn carries(&self) -> &Kinds {
        &self.carries
    }

    /// Whose units may board.
    #[inline]
    pub fn boarding(&self) -> Affiliation {
        self.boarding
    }

    /// What happens to the passengers when the holder dies.
    #[inline]
    pub fn passenger_fate(&self) -> PassengerFate {
        self.passenger_fate
    }

    /// What passengers do while aboard.
    #[inline]
    pub fn conduct(&self) -> PassengerConduct {
        self.conduct
    }
}
