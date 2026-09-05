//! Loading game content from a script into a [`ContentRegistry`].

use ferrets_content::{
    build::BuilderAttendance,
    field::{FieldAction, FieldAffiliation, FieldCoverage, FieldVision},
    location::Solidity,
    morph::{MorphCancel, MorphPlacement},
    projectile::Aim,
    registry::ContentRegistry,
    resource::DepletionPolicy,
    skills::EntityCastTarget,
    splash::SplashShape,
    stack_rule::StackRule,
    stats::ModifierOp,
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    turret::{TurretFire, WeaponConduct},
    work::WorkPresence,
};
use ferrets_math::FixedU64;

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

/// The error for a `what` that is none of `expected`, each entry already
/// rendered as the writer would read it (`'instant'`, `a { cycle = ... } table`),
/// and `found` rendered the same way.
pub(crate) fn unexpected(what: &str, expected: &[&str], found: &str) -> ScriptError {
    ScriptError::ContentError(format!(
        "{what} must be {}, found {found}",
        alternatives(expected)
    ))
}

/// A keyword as it appears in an error message.
pub(crate) fn quoted(value: &str) -> String {
    format!("'{value}'")
}

/// Joins alternatives the way a sentence lists them: `a`, `a or b`, `a, b, or c`.
fn alternatives(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [only] => (*only).to_string(),
        [first, second] => format!("{first} or {second}"),
        [leading @ .., last] => format!("{}, or {last}", leading.join(", ")),
    }
}

/// Resolves a keyword `value` to the option it names among `options`.
fn keyword<T: Copy>(what: &str, value: &str, options: &[(&str, T)]) -> crate::Result<T> {
    options
        .iter()
        .find(|(name, _)| *name == value)
        .map(|&(_, option)| option)
        .ok_or_else(|| {
            let names: Vec<String> = options.iter().map(|(name, _)| quoted(name)).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            unexpected(what, &names, &quoted(value))
        })
}

/// Maps a solidity name to its enum.
pub(crate) fn solidity(value: &str) -> crate::Result<Solidity> {
    keyword(
        "solidity",
        value,
        &[("solid", Solidity::Solid), ("passable", Solidity::Passable)],
    )
}

/// Maps an attack aim name to its enum.
pub(crate) fn attack_aim(value: &str) -> crate::Result<Aim> {
    keyword(
        "attack aim",
        value,
        &[("entity", Aim::Entity), ("position", Aim::Position)],
    )
}

/// Maps a splash shape name to its enum.
pub(crate) fn splash_shape(value: &str) -> crate::Result<SplashShape> {
    keyword(
        "splash shape",
        value,
        &[
            ("circular", SplashShape::Circular),
            ("line", SplashShape::Line),
        ],
    )
}

/// Maps a turret fire name to its enum.
pub(crate) fn turret_fire(value: &str) -> crate::Result<TurretFire> {
    keyword(
        "turret fire",
        value,
        &[("focus", TurretFire::Focus), ("spread", TurretFire::Spread)],
    )
}

/// Maps a weapon conduct name to its enum.
pub(crate) fn weapon_conduct(value: &str) -> crate::Result<WeaponConduct> {
    keyword(
        "weapon conduct",
        value,
        &[
            ("halts", WeaponConduct::Halts),
            ("on_the_move", WeaponConduct::OnTheMove),
        ],
    )
}

/// Maps a work presence name to its enum.
pub(crate) fn work_presence(value: &str) -> crate::Result<WorkPresence> {
    keyword(
        "work presence",
        value,
        &[
            ("hidden", WorkPresence::Hidden),
            ("present", WorkPresence::Present),
            ("present_stacking", WorkPresence::PresentStacking),
        ],
    )
}

/// Maps a builder-attendance name to its enum: a work presence the builder
/// keeps as crew, or a way of not staying.
pub(crate) fn builder_attendance(value: &str) -> crate::Result<BuilderAttendance> {
    keyword(
        "builder attendance",
        value,
        &[
            ("hidden", BuilderAttendance::Crew(WorkPresence::Hidden)),
            ("present", BuilderAttendance::Crew(WorkPresence::Present)),
            (
                "present_stacking",
                BuilderAttendance::Crew(WorkPresence::PresentStacking),
            ),
            ("unattended", BuilderAttendance::Unattended),
            ("consumed", BuilderAttendance::Consumed),
        ],
    )
}

/// Maps a boarding policy name to its enum.
pub(crate) fn boarding_policy(value: &str) -> crate::Result<BoardingPolicy> {
    keyword(
        "boarding policy",
        value,
        &[
            ("own", BoardingPolicy::Own),
            ("allies", BoardingPolicy::Allies),
        ],
    )
}

/// Maps a passenger fate name to its enum.
pub(crate) fn passenger_fate(value: &str) -> crate::Result<PassengerFate> {
    keyword(
        "passenger fate",
        value,
        &[
            ("destroy", PassengerFate::Destroy),
            ("eject", PassengerFate::Eject),
        ],
    )
}

/// Maps a passenger conduct name to its enum.
pub(crate) fn passenger_conduct(value: &str) -> crate::Result<PassengerConduct> {
    keyword(
        "passenger conduct",
        value,
        &[
            ("shelter", PassengerConduct::Shelter),
            ("fight", PassengerConduct::Fight),
        ],
    )
}

/// Maps a depletion policy name to its enum.
pub(crate) fn depletion(value: &str) -> crate::Result<DepletionPolicy> {
    keyword(
        "depletion policy",
        value,
        &[
            ("persist", DepletionPolicy::Persist),
            ("destroy", DepletionPolicy::Destroy),
        ],
    )
}

/// Maps a morph placement name to its enum.
pub(crate) fn morph_placement(value: &str) -> crate::Result<MorphPlacement> {
    keyword(
        "morph placement",
        value,
        &[
            ("reserve", MorphPlacement::Reserve),
            ("revalidate", MorphPlacement::Revalidate),
        ],
    )
}

/// Maps a morph cancel name to its enum.
pub(crate) fn morph_cancel(value: &str) -> crate::Result<MorphCancel> {
    keyword(
        "morph cancel",
        value,
        &[
            ("committed", MorphCancel::Committed),
            ("forfeit", MorphCancel::Forfeit),
            ("refundable", MorphCancel::Refundable),
        ],
    )
}

/// Maps a skill target name to its enum.
pub(crate) fn entity_cast_target(value: &str) -> crate::Result<EntityCastTarget> {
    keyword(
        "skill target",
        value,
        &[
            ("caster", EntityCastTarget::Caster),
            ("ally", EntityCastTarget::Ally),
            ("enemy", EntityCastTarget::Enemy),
            ("position", EntityCastTarget::Position),
        ],
    )
}

/// Maps a modifier op name to its enum.
pub(crate) fn modifier_op(value: &str) -> crate::Result<ModifierOp> {
    keyword(
        "modifier op",
        value,
        &[
            ("flat", ModifierOp::FlatAdd),
            ("percent", ModifierOp::PercentAdd),
        ],
    )
}

/// Maps a field vision name to its enum.
pub(crate) fn field_vision(value: &str) -> crate::Result<FieldVision> {
    keyword(
        "field vision",
        value,
        &[
            ("dark", FieldVision::Dark),
            ("watched", FieldVision::Watched),
        ],
    )
}

/// Maps a field affiliation name to its enum.
pub(crate) fn field_affiliation(value: &str) -> crate::Result<FieldAffiliation> {
    keyword(
        "field affiliation",
        value,
        &[
            ("own", FieldAffiliation::Own),
            ("allied", FieldAffiliation::Allied),
            ("anyone", FieldAffiliation::Anyone),
        ],
    )
}

/// Maps a field coverage name to its enum.
pub(crate) fn field_coverage(value: &str) -> crate::Result<FieldCoverage> {
    keyword(
        "field coverage",
        value,
        &[
            ("footprint", FieldCoverage::Footprint),
            ("anchor", FieldCoverage::Anchor),
        ],
    )
}

/// Maps a field action name to its enum.
pub(crate) fn field_action(value: &str) -> crate::Result<FieldAction> {
    keyword(
        "field action",
        value,
        &[("cover", FieldAction::Cover), ("clear", FieldAction::Clear)],
    )
}

/// Maps a stack-rule name to its enum: a fixed keyword, or `stack:N` for a cap
/// of `N` instances.
pub(crate) fn stack_rule(value: &str) -> crate::Result<StackRule> {
    match value {
        "refresh" => Ok(StackRule::Refresh),
        "ignore" => Ok(StackRule::Ignore),
        other => other
            .strip_prefix("stack:")
            .and_then(|cap| cap.parse::<u32>().ok())
            .map(StackRule::StackToCap)
            .ok_or_else(|| {
                unexpected(
                    "stack rule",
                    &["'refresh'", "'ignore'", "'stack:N'"],
                    &quoted(other),
                )
            }),
    }
}
