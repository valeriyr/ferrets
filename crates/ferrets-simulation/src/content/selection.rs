//! Content-defined selection property struct.

/// Content-defined selection properties for an entity type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionDef {
    /// Relative weight for picking the lead unit of a mixed selection: higher wins.
    priority: u32,
    /// The group instances share for select-all-of-type. `None` falls back to the
    /// type name, so each type is its own class unless content shares one
    /// explicitly; resolve it through
    /// [`EntityTypeDef::selection_class`](super::entity_type_def::EntityTypeDef::selection_class),
    /// which knows the name to fall back to.
    class: Option<String>,
}

impl SelectionDef {
    /// Creates a new `SelectionDef` with the given data.
    ///
    /// Panics if `class` is empty.
    pub fn new(priority: u32, class: Option<&str>) -> Self {
        assert!(
            class.is_none_or(|class| !class.is_empty()),
            "selection class must not be empty"
        );

        Self {
            priority,
            class: class.map(str::to_string),
        }
    }

    /// The lead-unit weight for a mixed selection.
    #[inline]
    pub fn priority(&self) -> u32 {
        self.priority
    }

    /// The explicitly declared select-all-of-type class, if any.
    #[inline]
    pub fn class(&self) -> Option<&str> {
        self.class.as_deref()
    }
}
