//! Content-defined production-catalogue property struct.

/// Content-defined production catalogue: which entity types this entity can train.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainerDef {
    trains: Vec<String>,
}

impl TrainerDef {
    /// Creates a new `TrainerDef` with the given data.
    ///
    /// Panics if `trains` is empty or contains an empty type name.
    pub fn new(trains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let trains: Vec<String> = trains.into_iter().map(Into::into).collect();

        assert!(!trains.is_empty(), "trains must not be empty");
        assert!(
            trains.iter().all(|name| !name.is_empty()),
            "trained type names must not be empty"
        );

        Self { trains }
    }

    /// Returns `true` if units of `type_name` can be trained here.
    pub fn can_train(&self, type_name: &str) -> bool {
        self.trains.iter().any(|name| name == type_name)
    }

    /// Returns the entity types that can be trained.
    pub fn trains(&self) -> impl Iterator<Item = &str> {
        self.trains.iter().map(String::as_str)
    }
}
