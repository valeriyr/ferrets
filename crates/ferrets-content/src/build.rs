//! Content-defined construction-catalogue property struct.

use crate::work::WorkPresence;

/// How a builder relates to a site it raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderAttendance {
    /// Joins the site's crew: its build order stays on the site and advances
    /// the progress one tick per tick, standing as the presence says.
    Crew(WorkPresence),
    /// Leaves the site unattended: its build order ends once the site is
    /// placed and paid for, and the site advances itself.
    Unattended,
    /// Works the site alone, hidden inside it, its build order advancing the
    /// progress; its supply is not counted while it is inside. It is consumed
    /// when the site completes instead of stepping back out, and a build order
    /// that ends early brings it back onto the map.
    Consumed,
}

/// Content-defined construction catalogue: which entity types this entity can
/// build, and how it attends the sites it raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderDef {
    /// The entity types this entity can construct.
    builds: Vec<String>,
    /// How the builder relates to a site it raises.
    attendance: BuilderAttendance,
}

impl BuilderDef {
    /// Creates a new `BuilderDef` with the given data.
    ///
    /// Panics if `builds` is empty or contains an empty type name.
    pub fn new(
        builds: impl IntoIterator<Item = impl Into<String>>,
        attendance: BuilderAttendance,
    ) -> Self {
        let builds: Vec<String> = builds.into_iter().map(Into::into).collect();

        assert!(!builds.is_empty(), "builds must not be empty");
        assert!(
            builds.iter().all(|name| !name.is_empty()),
            "constructed type names must not be empty"
        );

        Self { builds, attendance }
    }

    /// Returns `true` if buildings of `type_name` can be constructed by this entity.
    pub fn can_build(&self, type_name: &str) -> bool {
        self.builds.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be constructed.
    pub fn builds(&self) -> impl Iterator<Item = &str> {
        self.builds.iter().map(String::as_str)
    }

    /// How the builder relates to a site it raises.
    #[inline]
    pub fn attendance(&self) -> BuilderAttendance {
        self.attendance
    }
}
