//! Loading game content from a script into a [`ContentRegistry`].

use ferrets_math::FixedU64;
use ferrets_simulation::components::location::Solidity;
use ferrets_simulation::components::resource::{DepletionPolicy, HarvestVisibility};
use ferrets_simulation::content::entity_type_def::EntityTypeDef;
use ferrets_simulation::content::registry::ContentRegistry;

use crate::engine::ScriptEngine;
use crate::error::ScriptError;

/// One content declaration produced by a script.
pub enum Definition {
    Race(String),
    Resource(String),
    Entity(Box<EntityTypeDef>),
}

/// Loads content from `source` with the given engine, returning a validated
/// registry.
///
/// Script and field errors (bad syntax, wrong types, malformed numbers) are
/// returned. Content-consistency errors (an unregistered race, an invalid
/// production catalogue) panic, matching the registry's Rust API.
pub fn load(engine: &dyn ScriptEngine, source: &str) -> crate::Result<ContentRegistry> {
    let definitions = engine.load_content(source)?;

    let mut registry = ContentRegistry::default();
    for definition in definitions {
        match definition {
            Definition::Race(name) => registry.register_race(name),
            Definition::Resource(kind) => registry.register_resource(kind),
            Definition::Entity(def) => registry.register(*def),
        }
    }
    registry.validate();
    Ok(registry)
}

/// Parses a decimal string to fixed-point directly, without an intermediate
/// `f64`, so the result is identical on every platform.
pub(crate) fn fixed(value: &str) -> crate::Result<FixedU64> {
    value
        .parse::<FixedU64>()
        .map_err(|error| ScriptError::NumberError(format!("'{value}': {error}")))
}

/// Maps a solidity name to its enum.
pub(crate) fn solidity(value: &str) -> crate::Result<Solidity> {
    match value {
        "solid" => Ok(Solidity::Solid),
        "passable" => Ok(Solidity::Passable),
        other => Err(ScriptError::ContentError(format!(
            "unknown solidity '{other}'"
        ))),
    }
}

/// Maps a depletion-policy name to its enum.
pub(crate) fn depletion(value: &str) -> crate::Result<DepletionPolicy> {
    match value {
        "persist" => Ok(DepletionPolicy::Persist),
        "destroy" => Ok(DepletionPolicy::Destroy),
        other => Err(ScriptError::ContentError(format!(
            "unknown depletion policy '{other}'"
        ))),
    }
}

/// Maps a harvest-visibility name to its enum.
pub(crate) fn visibility(value: &str) -> crate::Result<HarvestVisibility> {
    match value {
        "hidden" => Ok(HarvestVisibility::Hidden),
        "visible" => Ok(HarvestVisibility::Visible),
        other => Err(ScriptError::ContentError(format!(
            "unknown harvest visibility '{other}'"
        ))),
    }
}
