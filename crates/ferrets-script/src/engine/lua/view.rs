//! Encoding the AI view snapshots as the Lua tables a think call receives.

use mlua::{Lua, Table, Value};

use crate::ai::view::content::{ContentView, EntityContentView};
use crate::ai::view::game::{EntityView, GameView};

/// Encodes a game view as the `view` table a think call receives.
pub(super) fn game_table(lua: &Lua, view: &GameView) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("tick", view.tick)?;
    table.set("player", view.player)?;
    table.set("race", view.race.as_str())?;

    let map = lua.create_table()?;
    map.set("width", view.map_width)?;
    map.set("height", view.map_height)?;
    table.set("map", map)?;

    let resources = lua.create_table()?;
    for (kind, amount) in &view.resources {
        resources.set(kind.as_str(), *amount)?;
    }
    table.set("resources", resources)?;

    let supply = lua.create_table()?;
    supply.set("provided", view.supply_provided)?;
    supply.set("used", view.supply_used)?;
    table.set("supply", supply)?;

    table.set("my_entities", entities_table(lua, &view.my_entities)?)?;
    table.set("ally_entities", entities_table(lua, &view.ally_entities)?)?;
    table.set("enemy_entities", entities_table(lua, &view.enemy_entities)?)?;
    table.set(
        "neutral_entities",
        entities_table(lua, &view.neutral_entities)?,
    )?;
    Ok(table)
}

/// Encodes entity views as an array table, preserving their order.
fn entities_table(lua: &Lua, entities: &[EntityView]) -> mlua::Result<Table> {
    let array = lua.create_table()?;
    for (index, entity) in entities.iter().enumerate() {
        array.set(index + 1, entity_table(lua, entity)?)?;
    }
    Ok(array)
}

fn entity_table(lua: &Lua, entity: &EntityView) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", entity.id)?;
    table.set("type_name", entity.type_name.as_str())?;
    table.set("x", entity.x)?;
    table.set("y", entity.y)?;
    table.set("health", entity.health)?;
    table.set("damage", entity.damage)?;
    table.set("armor", entity.armor)?;
    table.set("idle", entity.idle)?;
    table.set("hidden", entity.hidden)?;
    if let Some((kind, amount)) = &entity.carrying {
        let carrying = lua.create_table()?;
        carrying.set("kind", kind.as_str())?;
        carrying.set("amount", *amount)?;
        table.set("carrying", carrying)?;
    }
    table.set("train_queue", strings_table(lua, &entity.train_queue)?)?;
    table.set("under_construction", entity.under_construction)?;
    table.set("stance", entity.stance.as_deref())?;
    table.set("resource_amount", entity.resource_amount)?;
    Ok(table)
}

/// Encodes the content catalogue as the read-only-by-convention `content` global.
pub(super) fn content_table(lua: &Lua, content: &ContentView) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("resources", strings_table(lua, &content.resources)?)?;

    let entities = lua.create_table()?;
    for entity in &content.entities {
        entities.set(entity.name.as_str(), entity_content_table(lua, entity)?)?;
    }
    table.set("entities", entities)?;
    Ok(table)
}

fn entity_content_table(lua: &Lua, entity: &EntityContentView) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    let cost = lua.create_table()?;
    for (index, (kind, amount)) in entity.cost.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("kind", kind.as_str())?;
        entry.set("amount", *amount)?;
        cost.set(index + 1, entry)?;
    }
    table.set("cost", cost)?;

    table.set("train_time", entity.train_time)?;
    table.set("build_time", entity.build_time)?;
    table.set("trains", optional_strings(lua, entity.trains.as_deref())?)?;
    table.set("builds", optional_strings(lua, entity.builds.as_deref())?)?;

    let size = lua.create_table()?;
    size.set("w", entity.size.0)?;
    size.set("h", entity.size.1)?;
    table.set("size", size)?;

    table.set("max_health", entity.max_health)?;
    if let Some(attack) = &entity.attack {
        let attack_table = lua.create_table()?;
        attack_table.set("damage", attack.damage)?;
        attack_table.set("attack_range", attack.attack_range)?;
        table.set("attack", attack_table)?;
    }
    table.set(
        "harvests",
        optional_strings(lua, entity.harvests.as_deref())?,
    )?;
    table.set("stores", optional_strings(lua, entity.stores.as_deref())?)?;
    table.set("can_move", entity.can_move)?;
    Ok(table)
}

/// Encodes strings as an array table, preserving their order.
fn strings_table(lua: &Lua, strings: &[String]) -> mlua::Result<Table> {
    let array = lua.create_table()?;
    for (index, value) in strings.iter().enumerate() {
        array.set(index + 1, value.as_str())?;
    }
    Ok(array)
}

/// Like [`strings_table`], mapping `None` to `nil`.
fn optional_strings(lua: &Lua, strings: Option<&[String]>) -> mlua::Result<Value> {
    match strings {
        Some(strings) => Ok(Value::Table(strings_table(lua, strings)?)),
        None => Ok(Value::Nil),
    }
}
