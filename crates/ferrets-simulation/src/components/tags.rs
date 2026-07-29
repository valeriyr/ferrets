//! Content-declared classification tags for simulation entities.

use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

/// The classification tags an entity carries. Tag names are free-form.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct TagsComponent {
    tags: BTreeSet<String>,
}

impl TagsComponent {
    /// Creates a tag set from the given tag names.
    pub fn new(tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            tags: tags.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns `true` if the entity carries `tag`.
    pub fn contains(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }
}
