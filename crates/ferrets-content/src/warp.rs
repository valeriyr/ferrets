//! Content-defined warp-in property struct.

use crate::{field::FieldPlacement, landing::Landing};

/// Content-defined warp-in catalogue: which entity types this entity can warp
/// in, and onto which cells. A unit warped in comes into the world at once on
/// a cell its player picks, rising over its own rise time, and the warper
/// cools down for the unit's train time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarperDef {
    /// The types it warps in, by name.
    warps: Vec<String>,
    /// The field rules the picked cells must meet for the unit's owner, held
    /// until it has risen.
    within: Vec<FieldPlacement>,
    /// Which cells the owner's side must know of to warp onto.
    landing: Landing,
}

impl WarperDef {
    /// Creates a new `WarperDef` with the given data.
    ///
    /// Panics if `warps` is empty or contains an empty type name.
    pub fn new(
        warps: impl IntoIterator<Item = impl Into<String>>,
        within: Vec<FieldPlacement>,
        landing: Landing,
    ) -> Self {
        let warps: Vec<String> = warps.into_iter().map(Into::into).collect();

        assert!(!warps.is_empty(), "warps must not be empty");
        assert!(
            warps.iter().all(|name| !name.is_empty()),
            "warped type names must not be empty"
        );

        Self {
            warps,
            within,
            landing,
        }
    }

    /// Returns `true` if units of `type_name` can be warped in here.
    pub fn can_warp(&self, type_name: &str) -> bool {
        self.warps.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be warped in.
    pub fn warps(&self) -> impl Iterator<Item = &str> {
        self.warps.iter().map(String::as_str)
    }

    /// The field rules the picked cells must meet for the unit's owner.
    pub fn within(&self) -> &[FieldPlacement] {
        &self.within
    }

    /// Which cells the owner's side must know of to warp onto.
    pub fn landing(&self) -> Landing {
        self.landing
    }
}
