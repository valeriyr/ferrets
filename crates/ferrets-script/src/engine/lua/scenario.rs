//! The scenario-DSL binding: a hardened, session-long Lua state hosting one
//! scenario's objectives and win/loss check.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::ai::view::content::ContentView;
use crate::ai::view::game::GameView;
use crate::engine::lua::{self, view};
use crate::error::ScriptError;
use crate::scenario::{ObjectiveStatus, Outcome, ScenarioRuntime, ScenarioStatus};

/// A live Lua evaluator for one scenario.
///
/// The wrapped Lua state is single-threaded (`!Send`); callers own keeping it
/// on one thread.
pub(super) struct LuaScenarioRuntime {
    lua: Lua,
    name: String,
    period: u32,
    objectives: Vec<Objective>,
    check: Function,
    state: Table,
}

impl LuaScenarioRuntime {
    /// Creates the runtime: builds a hardened Lua state, installs the `content`
    /// global from `content`, registers `define_scenario`, and executes
    /// `source`.
    ///
    /// Errors if the script fails, does not call `define_scenario` exactly
    /// once, or declares an invalid definition (a non-function `check`, a
    /// `period` that is not a positive integer, malformed objectives).
    pub(super) fn new(source: &str, content: &ContentView) -> crate::Result<LuaScenarioRuntime> {
        let lua = Lua::new();
        lua::harden(&lua).map_err(lua::engine_error)?;
        let content_table = view::content_table(&lua, content).map_err(lua::engine_error)?;
        lua.globals()
            .set("content", content_table)
            .map_err(lua::engine_error)?;

        let definition: Rc<RefCell<Option<ScenarioDefinition>>> = Rc::new(RefCell::new(None));
        register_define_scenario(&lua, &definition).map_err(lua::engine_error)?;
        lua.load(source).exec().map_err(lua::from_lua_error)?;

        let Some(definition) = definition.borrow_mut().take() else {
            return Err(ScriptError::ScenarioError(
                "script must call define_scenario".to_string(),
            ));
        };
        let state = lua.create_table().map_err(lua::engine_error)?;

        Ok(LuaScenarioRuntime {
            lua,
            name: definition.name,
            period: definition.period,
            objectives: definition.objectives,
            check: definition.check,
            state,
        })
    }
}

impl ScenarioRuntime for LuaScenarioRuntime {
    fn name(&self) -> &str {
        &self.name
    }

    fn period(&self) -> u32 {
        self.period
    }

    fn evaluate(&mut self, view: &GameView) -> crate::Result<ScenarioStatus> {
        let view_table = view::game_table(&self.lua, view).map_err(lua::engine_error)?;
        let result: Value = self
            .check
            .call((self.state.clone(), view_table))
            .map_err(lua::from_lua_error)?;
        parse_status(&self.objectives, result)
    }
}

/// An objective's identity and display label, as declared by the script.
struct Objective {
    id: String,
    label: String,
}

/// What one `define_scenario` call declared.
struct ScenarioDefinition {
    name: String,
    period: u32,
    objectives: Vec<Objective>,
    check: Function,
}

/// Builds a [`ScenarioStatus`] from what `check` returned, in declared
/// objective order.
///
/// `check` must return a table `{ objectives = { <id> = bool, .. }, outcome =
/// "ongoing" | "victory" | "defeat" }`. A declared objective absent from the
/// returned `objectives` table counts as not met; an absent `outcome` counts as
/// `"ongoing"`.
fn parse_status(objectives: &[Objective], result: Value) -> crate::Result<ScenarioStatus> {
    let table = match result {
        Value::Table(table) => table,
        other => {
            return Err(ScriptError::ScenarioError(format!(
                "check must return a table, got {}",
                other.type_name()
            )));
        }
    };

    let done = match table
        .get::<Value>("objectives")
        .map_err(lua::from_lua_error)?
    {
        Value::Table(done) => Some(done),
        Value::Nil => None,
        other => {
            return Err(ScriptError::ScenarioError(format!(
                "'objectives' must be a table, got {}",
                other.type_name()
            )));
        }
    };

    let mut statuses = Vec::with_capacity(objectives.len());
    for objective in objectives {
        let met = match &done {
            Some(done) => match done
                .get::<Value>(objective.id.as_str())
                .map_err(lua::from_lua_error)?
            {
                Value::Boolean(met) => met,
                Value::Nil => false,
                other => {
                    return Err(ScriptError::ScenarioError(format!(
                        "objective '{}' must be a boolean, got {}",
                        objective.id,
                        other.type_name()
                    )));
                }
            },
            None => false,
        };
        statuses.push(ObjectiveStatus {
            id: objective.id.clone(),
            label: objective.label.clone(),
            done: met,
        });
    }

    let outcome = match table.get::<Value>("outcome").map_err(lua::from_lua_error)? {
        Value::Nil => Outcome::Ongoing,
        Value::String(outcome) => match &*outcome.to_str().map_err(lua::from_lua_error)? {
            "ongoing" => Outcome::Ongoing,
            "victory" => Outcome::Victory,
            "defeat" => Outcome::Defeat,
            other => {
                return Err(ScriptError::ScenarioError(format!(
                    "'outcome' must be \"ongoing\", \"victory\", or \"defeat\", got \"{other}\""
                )));
            }
        },
        other => {
            return Err(ScriptError::ScenarioError(format!(
                "'outcome' must be a string, got {}",
                other.type_name()
            )));
        }
    };

    Ok(ScenarioStatus {
        objectives: statuses,
        outcome,
    })
}

/// Installs the `define_scenario` global, capturing exactly one definition into
/// `sink`.
fn register_define_scenario(
    lua: &Lua,
    sink: &Rc<RefCell<Option<ScenarioDefinition>>>,
) -> mlua::Result<()> {
    let sink = Rc::clone(sink);
    let define_scenario = lua.create_function(move |_, (name, options): (String, Table)| {
        let mut slot = sink.borrow_mut();
        if slot.is_some() {
            return Err(scenario_error(
                "define_scenario must be called exactly once",
            ));
        }
        if name.is_empty() {
            return Err(scenario_error("the scenario name must not be empty"));
        }

        let check = match options.get::<Value>("check")? {
            Value::Function(function) => function,
            other => {
                return Err(scenario_error(&format!(
                    "'check' must be a function, got {}",
                    other.type_name()
                )));
            }
        };
        let period = lua::parse_period(&options, scenario_error)?;
        let objectives = parse_objectives(&options)?;

        *slot = Some(ScenarioDefinition {
            name,
            period,
            objectives,
            check,
        });
        Ok(())
    })?;
    lua.globals().set("define_scenario", define_scenario)
}

/// Reads the ordered `objectives` list from a `define_scenario` options table.
fn parse_objectives(options: &Table) -> mlua::Result<Vec<Objective>> {
    let list = match options.get::<Value>("objectives")? {
        Value::Table(list) => list,
        other => {
            return Err(scenario_error(&format!(
                "'objectives' must be a list, got {}",
                other.type_name()
            )));
        }
    };

    let mut objectives = Vec::new();
    for element in list.sequence_values::<Table>() {
        let element = element?;
        let id: String = match element.get::<Value>("id")? {
            Value::String(id) => id.to_str()?.to_string(),
            other => {
                return Err(scenario_error(&format!(
                    "each objective needs a string 'id', got {}",
                    other.type_name()
                )));
            }
        };
        if id.is_empty() {
            return Err(scenario_error("an objective 'id' must not be empty"));
        }
        let label: String = match element.get::<Value>("label")? {
            Value::String(label) => label.to_str()?.to_string(),
            other => {
                return Err(scenario_error(&format!(
                    "objective '{id}' needs a string 'label', got {}",
                    other.type_name()
                )));
            }
        };
        objectives.push(Objective { id, label });
    }
    Ok(objectives)
}

/// A [`ScriptError::ScenarioError`] wrapped for raising from a Lua callback.
fn scenario_error(message: &str) -> mlua::Error {
    mlua::Error::external(ScriptError::ScenarioError(message.to_string()))
}
