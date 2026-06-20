//! In-flight construction state and content-defined builder properties for
//! simulation entities.

use bevy_ecs::prelude::*;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::simulation_id::SimulationId;

/// Content-defined construction catalogue: which entity types this entity can build.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct BuilderStaticData {
    builds: Vec<String>,
}

impl BuilderStaticData {
    /// Creates a new `BuilderStaticData` with the given data.
    ///
    /// Panics if `builds` is empty or contains an empty type name.
    pub fn new(builds: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let builds: Vec<String> = builds.into_iter().map(Into::into).collect();

        assert!(!builds.is_empty(), "builds must not be empty");
        assert!(
            builds.iter().all(|name| !name.is_empty()),
            "constructed type names must not be empty"
        );

        Self { builds }
    }

    /// Returns `true` if buildings of `type_name` can be constructed by this entity.
    pub fn can_build(&self, type_name: &str) -> bool {
        self.builds.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be constructed.
    pub fn builds(&self) -> impl Iterator<Item = &str> {
        self.builds.iter().map(String::as_str)
    }
}

/// Marks a building whose construction is still in progress.
#[derive(Component, Debug, Default)]
pub struct UnderConstructionComponent;

/// Per-entity in-flight construction state.
#[derive(Component, Debug, Default)]
pub struct BuildComponent {
    /// The building being constructed, once it has been placed on the map.
    pub building: Option<SimulationId>,
    /// Ticks spent constructing.
    pub progress: u32,
    /// `(own position, site position)` when the last chase started. Both
    /// unchanged on resume means the chase made no progress and never will.
    pub last_chase: Option<(FixedUVec2, FixedUVec2)>,
}
