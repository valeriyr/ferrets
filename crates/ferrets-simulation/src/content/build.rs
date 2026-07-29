//! Content-defined construction-catalogue property struct.

/// Content-defined construction catalogue: which entity types this entity can build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderDef {
    builds: Vec<String>,
}

impl BuilderDef {
    /// Creates a new `BuilderDef` with the given data.
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
