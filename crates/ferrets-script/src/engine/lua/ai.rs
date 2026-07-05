//! The AI-DSL binding: a hardened, session-long Lua state hosting one brain.

use std::cell::RefCell;
use std::rc::Rc;

use ferrets_simulation::command::PlayerCommand;
use mlua::{Function, Lua, Table, Value};

use crate::ai::AiRuntime;
use crate::ai::view::content::ContentView;
use crate::ai::view::game::GameView;
use crate::engine::lua::{self, command, view};
use crate::error::ScriptError;

/// A live Lua brain for one AI player.
///
/// The wrapped Lua state is single-threaded (`!Send`); callers own keeping it
/// on one thread.
pub(super) struct LuaAiRuntime {
    lua: Lua,
    name: String,
    period: u32,
    think: Function,
    state: Table,
}

impl LuaAiRuntime {
    /// Creates the runtime: builds a hardened Lua state, installs the `content`
    /// global from `content`, registers `define_ai`, and executes `source`.
    ///
    /// Errors if the script fails, does not call `define_ai` exactly once, or
    /// declares an invalid definition (a non-function `think`, a `period` that
    /// is not a positive integer).
    pub(super) fn new(source: &str, content: &ContentView) -> crate::Result<LuaAiRuntime> {
        let lua = Lua::new();
        lua::harden(&lua).map_err(lua::engine_error)?;
        let content_table = view::content_table(&lua, content).map_err(lua::engine_error)?;
        lua.globals()
            .set("content", content_table)
            .map_err(lua::engine_error)?;

        let definition: Rc<RefCell<Option<AiDefinition>>> = Rc::new(RefCell::new(None));
        register_define_ai(&lua, &definition).map_err(lua::engine_error)?;
        lua.load(source).exec().map_err(lua::from_lua_error)?;

        let Some(definition) = definition.borrow_mut().take() else {
            return Err(ScriptError::AiError(
                "script must call define_ai".to_string(),
            ));
        };
        let state = lua.create_table().map_err(lua::engine_error)?;

        Ok(LuaAiRuntime {
            lua,
            name: definition.name,
            period: definition.period,
            think: definition.think,
            state,
        })
    }
}

impl AiRuntime for LuaAiRuntime {
    fn name(&self) -> &str {
        &self.name
    }

    fn period(&self) -> u32 {
        self.period
    }

    fn think(&mut self, view: &GameView) -> crate::Result<Vec<PlayerCommand>> {
        let view_table = view::game_table(&self.lua, view).map_err(lua::engine_error)?;
        let result: Value = self
            .think
            .call((self.state.clone(), view_table))
            .map_err(lua::from_lua_error)?;
        command::parse(result)
    }
}

/// What one `define_ai` call declared.
struct AiDefinition {
    name: String,
    period: u32,
    think: Function,
}

/// Installs the `define_ai` global, capturing exactly one definition into `sink`.
fn register_define_ai(lua: &Lua, sink: &Rc<RefCell<Option<AiDefinition>>>) -> mlua::Result<()> {
    let sink = Rc::clone(sink);
    let define_ai = lua.create_function(move |_, (name, options): (String, Table)| {
        let mut slot = sink.borrow_mut();
        if slot.is_some() {
            return Err(ai_error("define_ai must be called exactly once"));
        }
        if name.is_empty() {
            return Err(ai_error("the ai name must not be empty"));
        }

        let think = match options.get::<Value>("think")? {
            Value::Function(function) => function,
            other => {
                return Err(ai_error(&format!(
                    "'think' must be a function, got {}",
                    other.type_name()
                )));
            }
        };
        let period = match options.get::<Value>("period")? {
            Value::Integer(period) if period >= 1 => u32::try_from(period)
                .map_err(|_| ai_error(&format!("'period' {period} out of range")))?,
            // Integral floats pass everywhere else on the boundary (integer
            // division yields floats), so they pass here too.
            Value::Number(period)
                if period.fract() == 0.0 && (1.0..=u32::MAX as f64).contains(&period) =>
            {
                period as u32
            }
            other => {
                return Err(ai_error(&format!(
                    "'period' must be a positive integer, got {other:?}"
                )));
            }
        };

        *slot = Some(AiDefinition {
            name,
            period,
            think,
        });
        Ok(())
    })?;
    lua.globals().set("define_ai", define_ai)
}

/// An [`ScriptError::AiError`] wrapped for raising from a Lua callback.
fn ai_error(message: &str) -> mlua::Error {
    mlua::Error::external(ScriptError::AiError(message.to_string()))
}
