//! Which entities a rule names — the one filter every capability spells the
//! same way.

use std::collections::BTreeSet;

use crate::entity_type_def::EntityTypeDef;

/// One name a filter lists, and the vocabulary it is drawn from.
///
/// Types and tags are separate namespaces and may share a name, so which one an
/// entry means is said rather than guessed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A registered entity type, by name.
    Type(String),
    /// A registered classification tag.
    Tag(String),
}

impl Kind {
    /// Whether an entity of this type is this kind.
    pub fn names(&self, def: &EntityTypeDef) -> bool {
        match self {
            Kind::Type(name) => *name == def.name,
            Kind::Tag(tag) => def.tags.contains(tag),
        }
    }

    /// The name itself, whichever vocabulary it is drawn from.
    pub fn name(&self) -> &str {
        match self {
            Kind::Type(name) | Kind::Tag(name) => name,
        }
    }

    /// The entry as a message names it, vocabulary and all.
    pub fn describe(&self) -> String {
        match self {
            Kind::Type(name) => format!("entity type '{name}'"),
            Kind::Tag(tag) => format!("tag '{tag}'"),
        }
    }
}

/// Which entities a rule names: registered entity types, tags, or a mix of
/// both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kinds {
    /// Anything at all.
    Any,
    /// Only what these name. An entity is named when any one entry names it.
    Only(BTreeSet<Kind>),
}

impl Kinds {
    /// Creates a `Kinds` naming exactly what `kinds` lists.
    ///
    /// Panics if `kinds` is empty — what names nothing is content that can
    /// never act — if any name is empty, or if an entry is named twice.
    pub fn only(kinds: impl IntoIterator<Item = Kind>) -> Self {
        let mut named: BTreeSet<Kind> = BTreeSet::new();
        for kind in kinds {
            assert!(!kind.name().is_empty(), "a kind name must not be empty");
            assert!(
                named.insert(kind.clone()),
                "kinds name {} twice",
                kind.describe()
            );
        }
        assert!(!named.is_empty(), "kinds must name something");

        Kinds::Only(named)
    }

    /// Creates a `Kinds` naming the given entity types.
    pub fn types(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Kinds::only(names.into_iter().map(|name| Kind::Type(name.into())))
    }

    /// Creates a `Kinds` naming the given tags.
    pub fn tags(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Kinds::only(names.into_iter().map(|name| Kind::Tag(name.into())))
    }

    /// Whether an entity of this type is one of these.
    pub fn admits(&self, def: &EntityTypeDef) -> bool {
        match self {
            Kinds::Any => true,
            Kinds::Only(kinds) => kinds.iter().any(|kind| kind.names(def)),
        }
    }

    /// The entries, in order. Empty for [`Any`](Self::Any), which lists nothing
    /// because it excludes nothing.
    pub fn kinds(&self) -> impl Iterator<Item = &Kind> {
        let named = match self {
            Kinds::Any => None,
            Kinds::Only(kinds) => Some(kinds),
        };
        named.into_iter().flatten()
    }
}
