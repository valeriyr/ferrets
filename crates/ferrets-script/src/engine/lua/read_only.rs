//! Read-only Lua tables: entries built on every read, every change refused.

use mlua::{Lua, MaybeSend, Table, Value};

/// A table named `name` that holds nothing itself: reading a key returns what
/// `entry` builds for it, fresh on every read, and a key `entry` has nothing
/// for raises; writing a key or changing its metatable raises too.
pub(super) fn table<F>(lua: &Lua, name: &'static str, entry: F) -> mlua::Result<Table>
where
    F: Fn(&Lua, &str) -> Option<mlua::Result<Value>> + MaybeSend + 'static,
{
    let metatable = lua.create_table()?;
    metatable.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, Value)| {
            let built = match &key {
                Value::String(text) => entry(lua, &text.to_string_lossy()),
                _ => None,
            };
            built.unwrap_or_else(|| {
                Err(mlua::Error::RuntimeError(format!(
                    "{name} has no entry {}",
                    key_name(&key)
                )))
            })
        })?,
    )?;
    metatable.set(
        "__newindex",
        lua.create_function(move |_, _: (Table, Value, Value)| -> mlua::Result<()> {
            Err(mlua::Error::RuntimeError(format!("{name} is read-only")))
        })?,
    )?;
    metatable.set("__metatable", format!("{name} is read-only"))?;
    let table = lua.create_table()?;
    table.set_metatable(Some(metatable))?;
    Ok(table)
}

/// How an error names `key`: a string quoted, anything else by its type.
fn key_name(key: &Value) -> String {
    match key {
        Value::String(text) => format!("'{}'", text.to_string_lossy()),
        other => other.type_name().to_string(),
    }
}
