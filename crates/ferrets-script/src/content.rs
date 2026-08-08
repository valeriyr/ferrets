//! Loading game content from a script into a [`ContentRegistry`].

use ferrets_math::FixedU64;
use ferrets_simulation::content::{
    location::Solidity, projectile::Aim, registry::ContentRegistry, resource::DepletionPolicy,
    splash::SplashShape, work::WorkPresence,
};

use crate::{engine::ScriptEngine, error::ScriptError};

/// Loads content from `source` with the given engine, returning a validated
/// registry.
///
/// Script and field errors (bad syntax, wrong types, malformed numbers) are
/// returned. Content-consistency errors (an unregistered race, an invalid
/// production catalogue) panic, matching the registry's Rust API.
pub fn load(engine: &dyn ScriptEngine, source: &str) -> crate::Result<ContentRegistry> {
    let registry = engine.load_content(source)?;
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

/// Maps an attack-aim name to its enum.
pub(crate) fn attack_aim(value: &str) -> crate::Result<Aim> {
    match value {
        "entity" => Ok(Aim::Entity),
        "position" => Ok(Aim::Position),
        other => Err(ScriptError::ContentError(format!(
            "unknown attack aim '{other}'"
        ))),
    }
}

/// Maps a splash-shape name to its enum.
pub(crate) fn splash_shape(value: &str) -> crate::Result<SplashShape> {
    match value {
        "circular" => Ok(SplashShape::Circular),
        "line" => Ok(SplashShape::Line),
        other => Err(ScriptError::ContentError(format!(
            "unknown splash shape '{other}'"
        ))),
    }
}

/// Maps a work-presence name to its enum.
pub(crate) fn work_presence(value: &str) -> crate::Result<WorkPresence> {
    match value {
        "hidden" => Ok(WorkPresence::Hidden),
        "present" => Ok(WorkPresence::Present),
        "present_stacking" => Ok(WorkPresence::PresentStacking),
        other => Err(ScriptError::ContentError(format!(
            "unknown work presence '{other}'"
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
