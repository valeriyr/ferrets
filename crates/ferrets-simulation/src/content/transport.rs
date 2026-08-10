//! Content-defined transport capability: whom an entity carries, and on what
//! terms.

use std::collections::BTreeSet;

use crate::components::tags::TagsComponent;

/// Whose units a transporter admits aboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardingPolicy {
    /// Only the holder's own units.
    Own,
    /// The holder's own units and those of its allies.
    Allies,
}

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
    /// The admission list. Each entry names an entity type or a tag; a
    /// candidate matching none of them is refused.
    carries: BTreeSet<String>,
    /// Whose units may board.
    boarding: BoardingPolicy,
    /// What happens to the passengers when the holder dies.
    passenger_fate: PassengerFate,
    /// What passengers do while aboard.
    conduct: PassengerConduct,
}

impl TransporterDef {
    /// Creates a new `TransporterDef` with the given data.
    ///
    /// Panics if `carries` is empty or contains an empty name.
    pub fn new(
        carries: impl IntoIterator<Item = impl Into<String>>,
        boarding: BoardingPolicy,
        passenger_fate: PassengerFate,
        conduct: PassengerConduct,
    ) -> Self {
        let carries: BTreeSet<String> = carries.into_iter().map(Into::into).collect();

        assert!(!carries.is_empty(), "carries must not be empty");
        assert!(
            carries.iter().all(|name| !name.is_empty()),
            "carried names must not be empty"
        );

        Self {
            carries,
            boarding,
            passenger_fate,
            conduct,
        }
    }

    /// Returns `true` if a candidate with the given type name and tags is one
    /// this entity will carry.
    pub fn admits(&self, candidate_type: &str, candidate_tags: Option<&TagsComponent>) -> bool {
        self.carries.iter().any(|name| {
            let name = name.as_str();
            name == candidate_type || candidate_tags.is_some_and(|tags| tags.contains(name))
        })
    }

    /// Returns the admission list entries.
    pub fn carries(&self) -> impl Iterator<Item = &str> {
        self.carries.iter().map(String::as_str)
    }

    /// Whose units may board.
    #[inline]
    pub fn boarding(&self) -> BoardingPolicy {
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
