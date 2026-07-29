//! The content-DSL binding: `define_*` host functions and the table readers
//! that map an entity table onto the [`EntityTypeDef`] builder.

use std::cell::RefCell;
use std::rc::Rc;

use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::nav_size::NavSize;
use ferrets_simulation::components::buffs::{BuffDef, StackRule};
use ferrets_simulation::components::skills::{SkillDef, SkillEffect, SkillTarget};
use ferrets_simulation::components::stats::{Modifier, ModifierOp, StatId};
use ferrets_simulation::content::entity_type_def::EntityTypeDef;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::content::resource::HarvestData;
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

    let stats = Rc::clone(registry);
    globals.set(
        "define_stat",
        lua.create_function(move |_, name: String| {
            stats.borrow_mut().register_stat(name);
            Ok(())
        })?,
    )?;

    let buffs = Rc::clone(registry);
    globals.set(
        "define_buff",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let buff = parse_buff(&table, &buffs.borrow()).map_err(mlua::Error::external)?;
            buffs.borrow_mut().register_buff(name, buff);
            Ok(())
        })?,
    )?;

    let skills = Rc::clone(registry);
    globals.set(
        "define_skill",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let skill = parse_skill(&table, &skills.borrow()).map_err(mlua::Error::external)?;
            skills.borrow_mut().register_skill(name, skill);
            Ok(())
        })?,
    )?;

    let entities = Rc::clone(registry);
    globals.set(
        "define_entity",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let def =
                build_entity(&name, &table, &entities.borrow()).map_err(mlua::Error::external)?;
            entities.borrow_mut().register(def);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// Builds one entity type from its Lua table, mapping each present field onto the
/// corresponding [`EntityTypeDef`] builder.
fn build_entity(
    name: &str,
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityTypeDef> {
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

    if let Some(stats) = optional::<Table>(table, "stats")? {
        for (stat, value) in parse_stats(&stats, registry)? {
            def = def.with_stat(stat, value);
        }
        // A weapon may omit its acquisition range; the attack range is the default.
        if def.can_attack()
            && def.base_stat(StatId::ACQUIRE_RANGE).is_none()
            && let Some(range) = def.base_stat(StatId::ATTACK_RANGE)
        {
            def = def.with_stat(StatId::ACQUIRE_RANGE, range);
        }
    }
    if let Some(dying) = optional::<Table>(table, "dying")? {
        let time = required::<u32>(&dying, "time")?;
        let corpse = optional::<String>(&dying, "corpse")?;
        def = def.with_dying(time, corpse.as_deref());
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
    if let Some(selection) = optional::<Table>(table, "selection")? {
        let priority = optional::<u32>(&selection, "priority")?.unwrap_or(0);
        let class = optional::<String>(&selection, "class")?;
        def = def.with_selection(priority, class.as_deref());
    }
    if let Some(bonuses) = optional::<Table>(table, "bonus_damage_vs")? {
        def = def.with_bonus_damage_vs(pairs::<u32>(&bonuses, "bonus_damage_vs")?);
    }
    if let Some(skills) = optional::<Vec<String>>(table, "skills")? {
        let ids = skills
            .iter()
            .map(|name| {
                registry.skill(name).ok_or_else(|| {
                    ScriptError::ContentError(format!("skill '{name}' is not defined"))
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        def = def.with_skills(ids);
    }

    Ok(def)
}

/// Reads the flat `stats = { name = value }` table: each key is a registered stat
/// name (built-in, or content-declared with `define_stat`), each value a
/// non-negative integer or a decimal string. An unknown name is rejected — a
/// custom stat must be declared before it is set.
fn parse_stats(
    stats: &Table,
    registry: &ContentRegistry,
) -> crate::Result<Vec<(StatId, FixedU64)>> {
    let mut out = Vec::new();
    for pair in stats.pairs::<String, Value>() {
        let (name, value) = pair.map_err(|error| field_error("stats", error))?;
        let stat = registry
            .stat(&name)
            .ok_or_else(|| ScriptError::ContentError(format!("stat '{name}' is not defined")))?;
        out.push((stat, stat_value(&name, value)?));
    }
    Ok(out)
}

/// Reads one stat value: a non-negative integer, or a decimal string for a
/// fractional value (floats are rejected at the determinism boundary).
fn stat_value(name: &str, value: Value) -> crate::Result<FixedU64> {
    match value {
        Value::Integer(n) if n >= 0 => Ok(FixedU64::from_num(n)),
        Value::String(s) => content::fixed(&s.to_string_lossy()),
        other => Err(ScriptError::ContentError(format!(
            "stat '{name}' must be a non-negative integer or a decimal string, got {}",
            other.type_name()
        ))),
    }
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

/// Reads one skill: `{ cooldown, energy_cost?, target, effect }`.
fn parse_skill(table: &Table, registry: &ContentRegistry) -> crate::Result<SkillDef> {
    let energy_cost = match optional::<String>(table, "energy_cost")? {
        Some(cost) => content::fixed(&cost)?,
        None => FixedU64::ZERO,
    };
    Ok(SkillDef {
        cooldown: required::<u32>(table, "cooldown")?,
        energy_cost,
        target: parse_skill_target(&required::<String>(table, "target")?)?,
        effect: parse_skill_effect(&required::<Table>(table, "effect")?, registry)?,
    })
}

fn parse_skill_target(target: &str) -> crate::Result<SkillTarget> {
    match target {
        "self" => Ok(SkillTarget::Caster),
        "ally" => Ok(SkillTarget::Ally),
        "enemy" => Ok(SkillTarget::Enemy),
        other => Err(ScriptError::ContentError(format!(
            "unknown skill target '{other}' (expected self, ally, or enemy)"
        ))),
    }
}

/// Reads a skill effect: exactly one of `apply_buff`, `remove_buff`, `damage`, `heal`.
fn parse_skill_effect(table: &Table, registry: &ContentRegistry) -> crate::Result<SkillEffect> {
    if let Some(name) = optional::<String>(table, "apply_buff")? {
        let id = registry
            .buff(&name)
            .ok_or_else(|| ScriptError::ContentError(format!("buff '{name}' is not defined")))?;
        Ok(SkillEffect::ApplyBuff(id))
    } else if let Some(name) = optional::<String>(table, "remove_buff")? {
        let id = registry
            .buff(&name)
            .ok_or_else(|| ScriptError::ContentError(format!("buff '{name}' is not defined")))?;
        Ok(SkillEffect::RemoveBuff(id))
    } else if let Some(amount) = optional::<String>(table, "damage")? {
        Ok(SkillEffect::Damage(content::fixed(&amount)?))
    } else if let Some(amount) = optional::<String>(table, "heal")? {
        Ok(SkillEffect::Heal(content::fixed(&amount)?))
    } else {
        Err(ScriptError::ContentError(
            "skill effect must be one of apply_buff, remove_buff, damage, or heal".to_string(),
        ))
    }
}

/// Reads a buff definition: `{ duration?, stack, modifiers }`.
fn parse_buff(table: &Table, registry: &ContentRegistry) -> crate::Result<BuffDef> {
    Ok(BuffDef {
        duration: optional::<u32>(table, "duration")?,
        stack_rule: parse_stack_rule(&required::<String>(table, "stack")?)?,
        modifiers: parse_modifiers(&required::<Vec<Table>>(table, "modifiers")?, registry)?,
    })
}

fn parse_stack_rule(rule: &str) -> crate::Result<StackRule> {
    match rule {
        "refresh" => Ok(StackRule::Refresh),
        "ignore" => Ok(StackRule::Ignore),
        other => other
            .strip_prefix("stack:")
            .and_then(|cap| cap.parse::<u32>().ok())
            .map(StackRule::StackToCap)
            .ok_or_else(|| {
                ScriptError::ContentError(format!(
                    "unknown stack rule '{other}' (expected refresh, ignore, or stack:N)"
                ))
            }),
    }
}

fn parse_modifiers(
    modifiers: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<Modifier>> {
    modifiers
        .iter()
        .map(|modifier| parse_modifier(modifier, registry))
        .collect()
}

/// Reads one modifier: `{ stat, op, value }`, resolving the stat name to its id.
fn parse_modifier(table: &Table, registry: &ContentRegistry) -> crate::Result<Modifier> {
    let stat_name = required::<String>(table, "stat")?;
    let stat = registry
        .stat(&stat_name)
        .ok_or_else(|| ScriptError::ContentError(format!("stat '{stat_name}' is not defined")))?;
    let op = match required::<String>(table, "op")?.as_str() {
        "flat" => ModifierOp::FlatAdd,
        "percent" => ModifierOp::PercentAdd,
        other => {
            return Err(ScriptError::ContentError(format!(
                "unknown modifier op '{other}' (expected flat or percent)"
            )));
        }
    };
    let value = FixedI64::from_str(&required::<String>(table, "value")?)
        .map_err(|error| ScriptError::ContentError(format!("invalid modifier value: {error}")))?;
    Ok(Modifier {
        stat,
        op,
        magnitude: value,
    })
}
