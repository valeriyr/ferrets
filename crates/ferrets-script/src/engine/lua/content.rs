//! The content-DSL binding: `define_*` host functions and the table readers
//! that map an entity table onto the [`EntityTypeDef`] builder.

use std::cell::RefCell;
use std::rc::Rc;

use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::components::resource::HarvestData;
use ferrets_simulation::content::entity_type_def::EntityTypeDef;
use ferrets_simulation::content::registry::ContentRegistry;
use mlua::{Lua, Table, Value};

use crate::content;
use crate::error::ScriptError;

/// Installs the `define_*` globals, each registering into `registry` — the one
/// assigner of every derived id, so what a script observes (the layer id
/// `define_layer` returns) is what the finished registry holds.
pub(super) fn register(lua: &Lua, registry: &Rc<RefCell<ContentRegistry>>) -> mlua::Result<()> {
    let globals = lua.globals();

    let races = Rc::clone(registry);
    globals.set(
        "define_race",
        lua.create_function(move |_, name: String| {
            races.borrow_mut().register_race(name);
            Ok(())
        })?,
    )?;

    let resources = Rc::clone(registry);
    globals.set(
        "define_resource",
        lua.create_function(move |_, kind: String| {
            resources.borrow_mut().register_resource(kind);
            Ok(())
        })?,
    )?;

    let tags = Rc::clone(registry);
    globals.set(
        "define_tag",
        lua.create_function(move |_, tag: String| {
            tags.borrow_mut().register_tag(tag);
            Ok(())
        })?,
    )?;

    let layers = Rc::clone(registry);
    globals.set(
        "define_layer",
        lua.create_function(move |_, name: String| Ok(*layers.borrow_mut().register_layer(name)))?,
    )?;

    let lookup = Rc::clone(registry);
    globals.set(
        "layer_id",
        lua.create_function(move |_, name: String| match lookup.borrow().layer(&name) {
            Some(id) => Ok(*id),
            None => Err(mlua::Error::external(ScriptError::ContentError(format!(
                "layer '{name}' is not defined"
            )))),
        })?,
    )?;

    let terrains = Rc::clone(registry);
    globals.set(
        "define_terrain",
        lua.create_function(move |_, (name, passable): (String, u32)| {
            terrains.borrow_mut().register_terrain(name, passable);
            Ok(())
        })?,
    )?;

    let entities = Rc::clone(registry);
    globals.set(
        "define_entity",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let def = build_entity(&name, &table).map_err(mlua::Error::external)?;
            entities.borrow_mut().register(def);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// Builds one entity type from its Lua table, mapping each present field onto the
/// corresponding [`EntityTypeDef`] builder.
fn build_entity(name: &str, table: &Table) -> crate::Result<EntityTypeDef> {
    let mut def = EntityTypeDef::new(name);

    if let Some(race) = optional::<String>(table, "race")? {
        def = def.with_race(race);
    }

    let location = required::<Table>(table, "location")?;
    let occupation = required::<u32>(&location, "occupation")?;
    let solidity = required::<String>(&location, "solidity")?;
    def = def.with_location(
        occupation,
        nav_size(&location)?,
        content::solidity(&solidity)?,
    );

    if let Some(movement) = optional::<Table>(table, "movement")? {
        let speed = required::<String>(&movement, "speed")?;
        def = def.with_movement(content::fixed(&speed)?);
    }
    if let Some(health) = optional::<u32>(table, "health")? {
        def = def.with_health(health);
    }
    if let Some(dying) = optional::<Table>(table, "dying")? {
        let time = required::<u32>(&dying, "time")?;
        let corpse = optional::<String>(&dying, "corpse")?;
        def = def.with_dying(time, corpse.as_deref());
    }
    if let Some(attack) = optional::<Table>(table, "attack")? {
        let range = required::<u32>(&attack, "range")?;
        // Content may omit the acquisition range; the weapon range is the default.
        let acquire_range = optional::<u32>(&attack, "acquire_range")?.unwrap_or(range);
        def = def.with_attack(
            required::<u32>(&attack, "damage")?,
            range,
            acquire_range,
            required::<u32>(&attack, "aiming")?,
            required::<u32>(&attack, "reloading")?,
        );
    }
    if let Some(cost) = optional::<Table>(table, "cost")? {
        def = def.with_cost(pairs::<u32>(&cost, "cost")?);
    }
    if let Some(train_time) = optional::<u32>(table, "train_time")? {
        def = def.with_train_time(train_time);
    }
    if let Some(build_time) = optional::<u32>(table, "build_time")? {
        def = def.with_build_time(build_time);
    }
    if let Some(trainer) = optional::<Vec<String>>(table, "trainer")? {
        def = def.with_trainer(trainer);
    }
    if let Some(builder) = optional::<Vec<String>>(table, "builder")? {
        def = def.with_builder(builder);
    }
    if let Some(source) = optional::<Table>(table, "resource_source")? {
        let kind = required::<String>(&source, "kind")?;
        let depletion = required::<String>(&source, "depletion")?;
        def = def.with_resource_source(kind, content::depletion(&depletion)?);
    }
    if let Some(carrier) = optional::<Table>(table, "resource_carrier")? {
        def = def.with_resource_carrier(harvest_kinds(&carrier)?);
    }
    if let Some(storage) = optional::<Vec<String>>(table, "resource_storage")? {
        def = def.with_resource_storage(storage);
    }
    if let Some(tags) = optional::<Vec<String>>(table, "tags")? {
        def = def.with_tags(tags);
    }
    if let Some(priority) = optional::<i32>(table, "selection_priority")? {
        def = def.with_selection_priority(priority);
    }
    if let Some(class) = optional::<String>(table, "selection_class")? {
        def = def.with_selection_class(class);
    }
    if let Some(sight_range) = optional::<u32>(table, "sight_range")? {
        def = def.with_sight_range(sight_range);
    }

    Ok(def)
}

/// Reads `location.size`: an integer `n` means an `n×n` footprint; a `{w, h}`
/// array gives the two dimensions.
fn nav_size(location: &Table) -> crate::Result<NavSize> {
    match required::<Value>(location, "size")? {
        Value::Integer(side) => {
            let side = dimension(side)?;
            Ok(NavSize::new(side, side))
        }
        Value::Table(size) => {
            let width = index(&size, 1)?;
            let height = index(&size, 2)?;
            Ok(NavSize::new(width, height))
        }
        other => Err(ScriptError::ContentError(format!(
            "size must be an integer or {{width, height}}, got {}",
            other.type_name()
        ))),
    }
}

/// Reads a `{kind = amount}` map as `(kind, amount)` pairs.
fn pairs<V: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<Vec<(String, V)>> {
    let mut entries = Vec::new();
    for pair in table.clone().pairs::<String, V>() {
        entries.push(pair.map_err(|error| field_error(field, error))?);
    }
    Ok(entries)
}

/// Reads a `resource_carrier` map: each kind maps to `{capacity, time, visibility}`.
fn harvest_kinds(carrier: &Table) -> crate::Result<Vec<(String, HarvestData)>> {
    let mut carries = Vec::new();
    for pair in carrier.clone().pairs::<String, Table>() {
        let (kind, data) = pair.map_err(|error| field_error("resource_carrier", error))?;
        let harvest = HarvestData::new(
            required::<u32>(&data, "capacity")?,
            required::<u32>(&data, "time")?,
            content::visibility(&required::<String>(&data, "visibility")?)?,
        );
        carries.push((kind, harvest));
    }
    Ok(carries)
}

/// A required table field.
fn required<T: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<T> {
    let value: T = table
        .get(field)
        .map_err(|error| field_error(field, error))?;
    Ok(value)
}

/// An optional table field, `None` when absent.
fn optional<T: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<Option<T>> {
    let value: Option<T> = table
        .get(field)
        .map_err(|error| field_error(field, error))?;
    Ok(value)
}

/// A required positional (array) element.
fn index(table: &Table, position: i64) -> crate::Result<u32> {
    let value: i64 = table
        .get(position)
        .map_err(|error| ScriptError::ContentError(format!("element {position}: {error}")))?;
    dimension(value)
}

/// Narrows a Lua integer to a `u32` footprint dimension.
fn dimension(value: i64) -> crate::Result<u32> {
    u32::try_from(value)
        .map_err(|_| ScriptError::ContentError(format!("size {value} out of range")))
}

fn field_error(field: &str, error: mlua::Error) -> ScriptError {
    ScriptError::ContentError(format!("field '{field}': {error}"))
}
