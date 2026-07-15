//! The Lua binding of the scripting DSLs, backed by `mlua`.
//!
//! [`LuaEngine`] implements the [`ScriptEngine`] seam. Errors raised from
//! host functions round-trip as [`ScriptError`]s;
//! everything else the scripts observe — the `define_*` globals, the view
//! tables, the command schema — binds the engine-agnostic contracts documented
//! on [`content`](crate::content) and [`ai`](crate::ai).

mod ai;
mod command;
mod content;
mod scenario;
mod view;

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value, Variadic};

use crate::ai::view::content::ContentView;
use crate::content::Definition;
use crate::engine::ScriptEngine;
use crate::engine::lua::ai::LuaAiRuntime;
use crate::engine::lua::scenario::LuaScenarioRuntime;
use crate::error::ScriptError;

/// Loads content and AI scripts authored in Lua.
pub struct LuaEngine;

impl ScriptEngine for LuaEngine {
    fn load_content(&self, source: &str) -> crate::Result<Vec<Definition>> {
        let lua = Lua::new();
        harden(&lua).map_err(engine_error)?;
        let sink: Rc<RefCell<Vec<Definition>>> = Rc::new(RefCell::new(Vec::new()));

        content::register(&lua, &sink).map_err(engine_error)?;
        lua.load(source).exec().map_err(from_lua_error)?;

        let definitions = sink.borrow_mut().drain(..).collect();
        Ok(definitions)
    }

    fn load_ai(
        &self,
        source: &str,
        content: &ContentView,
    ) -> crate::Result<Box<dyn crate::ai::AiRuntime>> {
        Ok(Box::new(LuaAiRuntime::new(source, content)?))
    }

    fn load_scenario(
        &self,
        source: &str,
        content: &ContentView,
    ) -> crate::Result<Box<dyn crate::scenario::ScenarioRuntime>> {
        Ok(Box::new(LuaScenarioRuntime::new(source, content)?))
    }
}

/// Strips the ambient-state stdlib from a fresh state: `os` and `io` are
/// removed, and `math.random`/`math.randomseed` raise — scripts must stay
/// deterministic.
fn harden(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    globals.set("os", Value::Nil)?;
    globals.set("io", Value::Nil)?;

    let math: Table = globals.get("math")?;
    let unavailable = lua.create_function(|_, _: Variadic<Value>| -> mlua::Result<()> {
        Err(mlua::Error::external(ScriptError::EngineError(
            "math.random is unavailable; scripts must stay deterministic".to_string(),
        )))
    })?;
    math.set("random", unavailable.clone())?;
    math.set("randomseed", unavailable)?;
    Ok(())
}

fn engine_error(error: mlua::Error) -> ScriptError {
    ScriptError::EngineError(error.to_string())
}

/// Reads the positive-integer `period` a definition declares, raising via
/// `error` on anything else.
fn parse_period(options: &Table, error: impl Fn(&str) -> mlua::Error) -> mlua::Result<u32> {
    match options.get::<Value>("period")? {
        Value::Integer(period) if period >= 1 => {
            u32::try_from(period).map_err(|_| error(&format!("'period' {period} out of range")))
        }
        // Integral floats pass everywhere else on the boundary (integer
        // division yields floats), so they pass here too.
        Value::Number(period)
            if period.fract() == 0.0 && (1.0..=u32::MAX as f64).contains(&period) =>
        {
            Ok(period as u32)
        }
        other => Err(error(&format!(
            "'period' must be a positive integer, got {other:?}"
        ))),
    }
}

/// Recovers a [`ScriptError`] thrown from a host-function callback, or reports
/// the raw Lua failure as an engine error.
fn from_lua_error(error: mlua::Error) -> ScriptError {
    match &error {
        mlua::Error::CallbackError { cause, .. } => from_lua_error((**cause).clone()),
        mlua::Error::ExternalError(cause) => cause
            .downcast_ref::<ScriptError>()
            .cloned()
            .unwrap_or_else(|| ScriptError::EngineError(error.to_string())),
        _ => ScriptError::EngineError(error.to_string()),
    }
}
