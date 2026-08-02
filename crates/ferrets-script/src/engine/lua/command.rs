//! Parsing the command tables a think call returns into player commands.
//!
//! Results are read positionally (the array part only) and any malformed
//! element fails the whole batch — commands compose sequentially (a selection
//! followed by an order), so skipping one would silently misdirect the rest.

use std::collections::BTreeMap;

use ferrets_math::FixedU64;
use ferrets_math::fixed_urect::FixedURect;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::command::{PlayerCommand, SelectMode, SkillCasterRef};
use ferrets_simulation::components::rally::RallyTarget;
use ferrets_simulation::components::stance::Stance;
use ferrets_simulation::content::research::ResearchId;
use ferrets_simulation::content::skills::SkillId;
use ferrets_simulation::order::AttackTarget;
use ferrets_simulation::simulation_id::SimulationId;
use mlua::{Table, Value};

use crate::ai::view::content::ContentView;
use crate::error::ScriptError;

/// The name → handle indexes a command batch resolves against: scripts name
/// registered content, the commands they become carry ids.
pub(super) struct CommandNames {
    researches: BTreeMap<String, ResearchId>,
    skills: BTreeMap<String, SkillId>,
}

impl CommandNames {
    /// Builds the indexes from the content catalogue the runtime was given.
    pub(super) fn from_content(content: &ContentView) -> CommandNames {
        CommandNames {
            researches: content
                .researches
                .iter()
                .map(|research| (research.name.clone(), research.id))
                .collect(),
            skills: content
                .skills
                .iter()
                .map(|skill| (skill.name.clone(), skill.id))
                .collect(),
        }
    }
}

/// Parses the value a think call returned. `nil` means no commands.
/// `names` resolves the content names commands carry to their handles.
pub(super) fn parse(value: Value, names: &CommandNames) -> crate::Result<Vec<PlayerCommand>> {
    let table = match value {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(table) => table,
        other => {
            return Err(ScriptError::CommandError(format!(
                "think must return a command array or nil, got {}",
                other.type_name()
            )));
        }
    };

    let mut commands = Vec::new();
    for index in 1..=table.raw_len() {
        let element: Table = table
            .raw_get(index)
            .map_err(|error| element_error(index, &error.to_string()))?;
        commands.push(command(&element, index, names)?);
    }
    Ok(commands)
}

/// Parses one command table by its `kind` discriminator.
fn command(table: &Table, index: usize, names: &CommandNames) -> crate::Result<PlayerCommand> {
    let kind: String = field(table, index, "kind")?;
    match kind.as_str() {
        "select" => Ok(PlayerCommand::SelectById {
            id: SimulationId(integer(table, index, "id")?),
            mode: SelectMode::Replace,
        }),
        "select_area" => {
            let x1 = integer(table, index, "x1")?;
            let y1 = integer(table, index, "y1")?;
            let x2 = integer(table, index, "x2")?;
            let y2 = integer(table, index, "y2")?;
            // The range is inclusive in cells: the far corner reaches up to,
            // but not onto, the next cell's origin (containment includes the
            // rectangle's boundary, and resting units sit exactly on origins).
            let (far_x, far_y) = x2
                .checked_add(1)
                .zip(y2.checked_add(1))
                .ok_or_else(|| element_error(index, "cell range out of range"))?;
            let far = FixedUVec2::new(
                FixedU64::from_num(far_x) - FixedU64::DELTA,
                FixedU64::from_num(far_y) - FixedU64::DELTA,
            );
            Ok(PlayerCommand::SelectByRect {
                rect: FixedURect::from_corners(cell(x1, y1), far),
                mode: SelectMode::Replace,
            })
        }
        "move" => Ok(PlayerCommand::Move {
            target: cell(integer(table, index, "x")?, integer(table, index, "y")?),
            flush: flush(table, index)?,
        }),
        "attack" => Ok(PlayerCommand::Attack {
            target: attack_target(table, index)?,
            flush: flush(table, index)?,
        }),
        "attack_move" => Ok(PlayerCommand::AttackMove {
            target: cell(integer(table, index, "x")?, integer(table, index, "y")?),
            flush: flush(table, index)?,
        }),
        "patrol" => Ok(PlayerCommand::Patrol {
            target: cell(integer(table, index, "x")?, integer(table, index, "y")?),
            flush: flush(table, index)?,
        }),
        "guard" => Ok(PlayerCommand::Guard {
            target: SimulationId(integer(table, index, "target")?),
            flush: flush(table, index)?,
        }),
        "stance" => Ok(PlayerCommand::SetStance {
            stance: stance(table, index)?,
        }),
        "send" => Ok(PlayerCommand::SendToEntity {
            target: SimulationId(integer(table, index, "target")?),
            flush: flush(table, index)?,
        }),
        "train" => Ok(PlayerCommand::TrainEntity {
            trainer: SimulationId(integer(table, index, "trainer")?),
            type_name: field(table, index, "type_name")?,
        }),
        "research" => {
            let name: String = field(table, index, "research")?;
            let research = names.researches.get(&name).copied().ok_or_else(|| {
                field_error(index, "research", &format!("unknown research '{name}'"))
            })?;
            Ok(PlayerCommand::StartResearch {
                researcher: SimulationId(integer(table, index, "researcher")?),
                research,
            })
        }
        "use_skill" => {
            let name: String = field(table, index, "skill")?;
            let skill =
                names.skills.get(&name).copied().ok_or_else(|| {
                    field_error(index, "skill", &format!("unknown skill '{name}'"))
                })?;
            Ok(PlayerCommand::UseSkill {
                skill,
                caster: skill_caster(table, index)?,
                target: optional_integer(table, index, "target")?.map(SimulationId),
            })
        }
        "rally" => Ok(PlayerCommand::SetRallyPoint {
            entity: SimulationId(integer(table, index, "entity")?),
            target: rally_target(table, index)?,
        }),
        "build" => Ok(PlayerCommand::BuildEntity {
            builder: SimulationId(integer(table, index, "builder")?),
            type_name: field(table, index, "type_name")?,
            position: cell(integer(table, index, "x")?, integer(table, index, "y")?),
            flush: flush(table, index)?,
        }),
        "stop" => Ok(PlayerCommand::Stop),
        other => Err(element_error(index, &format!("unknown kind '{other}'"))),
    }
}

/// World position of the cell's origin corner.
fn cell(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::from(NavPos::new(x, y))
}

/// The `caster` of a use_skill command: an entity id, or the string
/// `"player"` for a player cast.
fn skill_caster(table: &Table, index: usize) -> crate::Result<SkillCasterRef> {
    let value: Value = table
        .raw_get("caster")
        .map_err(|error| field_error(index, "caster", &error.to_string()))?;
    match &value {
        Value::String(name) if name.to_string_lossy() == "player" => Ok(SkillCasterRef::Player),
        Value::String(other) => Err(field_error(
            index,
            "caster",
            &format!(
                "unknown caster '{}' (an entity id, or \"player\")",
                other.to_string_lossy()
            ),
        )),
        Value::Nil => Err(field_error(index, "caster", "missing")),
        _ => integer(table, index, "caster").map(|id| SkillCasterRef::Entity(SimulationId(id))),
    }
}

/// The `stance` field of a stance command, by name.
fn stance(table: &Table, index: usize) -> crate::Result<Stance> {
    let name: String = field(table, index, "stance")?;
    match name.as_str() {
        "flee" => Ok(Stance::Flee),
        "hold_fire" => Ok(Stance::HoldFire),
        "stand_ground" => Ok(Stance::StandGround),
        "defend" => Ok(Stance::Defend),
        other => Err(field_error(
            index,
            "stance",
            &format!("unknown stance '{other}'"),
        )),
    }
}

/// The rally target of a `rally` command: `target` names an entity, `x`/`y` a
/// cell, and neither clears the rally point. Mixing the two forms is an error.
fn rally_target(table: &Table, index: usize) -> crate::Result<Option<RallyTarget>> {
    let target = optional_integer(table, index, "target")?;
    let x = optional_integer(table, index, "x")?;
    let y = optional_integer(table, index, "y")?;
    match (target, x, y) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(element_error(
            index,
            "rally takes either a target or a cell, not both",
        )),
        (Some(id), None, None) => Ok(Some(RallyTarget::Entity(SimulationId(id)))),
        (None, Some(x), Some(y)) => Ok(Some(RallyTarget::Position(cell(x, y)))),
        (None, None, None) => Ok(None),
        (None, _, _) => Err(element_error(index, "rally cell needs both x and y")),
    }
}

/// The aim of an attack: either a `target` id or an `x`/`y` cell, never both.
fn attack_target(table: &Table, index: usize) -> crate::Result<AttackTarget> {
    let target = optional_integer(table, index, "target")?;
    let x = optional_integer(table, index, "x")?;
    let y = optional_integer(table, index, "y")?;
    match (target, x, y) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(element_error(
            index,
            "attack takes either a target or a cell, not both",
        )),
        (Some(id), None, None) => Ok(AttackTarget::Entity(SimulationId(id))),
        (None, Some(x), Some(y)) => Ok(AttackTarget::Position(cell(x, y))),
        (None, None, None) => Err(element_error(index, "attack needs a target or a cell")),
        (None, _, _) => Err(element_error(index, "attack cell needs both x and y")),
    }
}

/// A required field converted through `mlua`'s conversions.
fn field<T: mlua::FromLua>(table: &Table, index: usize, name: &str) -> crate::Result<T> {
    table
        .raw_get(name)
        .map_err(|error| field_error(index, name, &error.to_string()))
}

/// A required integer field. Integral floats are accepted (integer division in
/// a script yields floats); fractional values mean real float math leaked to
/// the boundary and are rejected.
fn integer(table: &Table, index: usize, name: &str) -> crate::Result<u32> {
    optional_integer(table, index, name)?.ok_or_else(|| field_error(index, name, "missing"))
}

/// An optional integer field: absent (`nil`) is `None`, present values follow
/// the same rules as [`integer`].
fn optional_integer(table: &Table, index: usize, name: &str) -> crate::Result<Option<u32>> {
    let value: Value = table
        .raw_get(name)
        .map_err(|error| field_error(index, name, &error.to_string()))?;
    match value {
        Value::Nil => Ok(None),
        Value::Integer(integer) => u32::try_from(integer)
            .map(Some)
            .map_err(|_| field_error(index, name, &format!("{integer} out of range"))),
        Value::Number(number)
            if number.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&number) =>
        {
            Ok(Some(number as u32))
        }
        Value::Number(number) => Err(field_error(
            index,
            name,
            &format!("{number} is not a whole number in range"),
        )),
        other => Err(field_error(
            index,
            name,
            &format!("expected integer, got {}", other.type_name()),
        )),
    }
}

/// The optional `flush` flag, `true` when absent.
fn flush(table: &Table, index: usize) -> crate::Result<bool> {
    let flush: Option<bool> = table
        .raw_get("flush")
        .map_err(|error| field_error(index, "flush", &error.to_string()))?;
    Ok(flush.unwrap_or(true))
}

fn field_error(index: usize, name: &str, message: &str) -> ScriptError {
    element_error(index, &format!("field '{name}': {message}"))
}

fn element_error(index: usize, message: &str) -> ScriptError {
    ScriptError::CommandError(format!("element {index}: {message}"))
}
