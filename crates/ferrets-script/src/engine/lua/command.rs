//! Parsing the command tables a think call returns into player commands.
//!
//! Results are read positionally (the array part only) and any malformed
//! element fails the whole batch — commands compose sequentially (a selection
//! followed by an order), so skipping one would silently misdirect the rest.

use ferrets_math::FixedU64;
use ferrets_math::fixed_urect::FixedURect;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::simulation_id::SimulationId;
use mlua::{Table, Value};

use crate::error::ScriptError;

/// Parses the value a think call returned. `nil` means no commands.
pub(super) fn parse(value: Value) -> crate::Result<Vec<PlayerCommand>> {
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
        commands.push(command(&element, index)?);
    }
    Ok(commands)
}

/// Parses one command table by its `kind` discriminator.
fn command(table: &Table, index: usize) -> crate::Result<PlayerCommand> {
    let kind: String = field(table, index, "kind")?;
    match kind.as_str() {
        "select" => Ok(PlayerCommand::SelectById {
            id: SimulationId(integer(table, index, "id")?),
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
            })
        }
        "move" => Ok(PlayerCommand::Move {
            target: cell(integer(table, index, "x")?, integer(table, index, "y")?),
            flush: flush(table, index)?,
        }),
        "attack" => Ok(PlayerCommand::Attack {
            target: SimulationId(integer(table, index, "target")?),
            flush: flush(table, index)?,
        }),
        "send" => Ok(PlayerCommand::SendToEntity {
            target: SimulationId(integer(table, index, "target")?),
            flush: flush(table, index)?,
        }),
        "train" => Ok(PlayerCommand::TrainEntity {
            trainer: SimulationId(integer(table, index, "trainer")?),
            type_name: field(table, index, "type_name")?,
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
    let value: Value = table
        .raw_get(name)
        .map_err(|error| field_error(index, name, &error.to_string()))?;
    match value {
        Value::Integer(integer) => u32::try_from(integer)
            .map_err(|_| field_error(index, name, &format!("{integer} out of range"))),
        Value::Number(number)
            if number.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&number) =>
        {
            Ok(number as u32)
        }
        Value::Number(number) => Err(field_error(
            index,
            name,
            &format!("{number} is not a whole number in range"),
        )),
        Value::Nil => Err(field_error(index, name, "missing")),
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
