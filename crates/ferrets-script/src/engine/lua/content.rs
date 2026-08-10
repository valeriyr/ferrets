//! The content-DSL binding: `define_*` host functions and the table readers
//! that map an entity table onto the [`EntityTypeDef`] builder.

use ferrets_geometry::cell_size::CellSize;
use std::{cell::RefCell, rc::Rc};

use ferrets_content::{
    costs::Cost,
    entity_buffs::EntityBuffDef,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    player_buffs::PlayerBuffDef,
    projectile::ProjectileDef,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    research::ResearchDef,
    resource::HarvestData,
    skills::{
        EntityCastCost, EntityCastEffect, EntityCastTarget, PlayerCastEffect, SkillCaster, SkillDef,
    },
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp, PlayerModifier},
};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_pathfinder::layer_mask::LayerMask;
use mlua::{Lua, Table, Value};

use crate::{content, error::ScriptError};

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
        "define_entity_stat",
        lua.create_function(move |_, name: String| {
            stats.borrow_mut().register_entity_stat(name);
            Ok(())
        })?,
    )?;

    let player_stats = Rc::clone(registry);
    globals.set(
        "define_player_stat",
        lua.create_function(move |_, name: String| {
            player_stats.borrow_mut().register_player_stat(name);
            Ok(())
        })?,
    )?;

    let entity_buffs = Rc::clone(registry);
    globals.set(
        "define_entity_buff",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let buff =
                parse_entity_buff(&table, &entity_buffs.borrow()).map_err(mlua::Error::external)?;
            entity_buffs.borrow_mut().register_entity_buff(name, buff);
            Ok(())
        })?,
    )?;

    let player_buffs = Rc::clone(registry);
    globals.set(
        "define_player_buff",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let buff =
                parse_player_buff(&table, &player_buffs.borrow()).map_err(mlua::Error::external)?;
            player_buffs.borrow_mut().register_player_buff(name, buff);
            Ok(())
        })?,
    )?;

    let projectiles = Rc::clone(registry);
    globals.set(
        "define_projectile",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let projectile = parse_projectile(&table).map_err(mlua::Error::external)?;
            projectiles
                .borrow_mut()
                .register_projectile(name, projectile);
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

    let researches = Rc::clone(registry);
    globals.set(
        "define_research",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let research =
                parse_research(&table, &researches.borrow()).map_err(mlua::Error::external)?;
            researches.borrow_mut().register_research(name, research);
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
        cell_size(&location)?,
        content::solidity(&solidity)?,
    );

    if let Some(stats) = optional::<Table>(table, "stats")? {
        for (stat, value) in parse_stats(&stats, registry)? {
            def = def.with_stat(stat, value);
        }
        // A weapon may omit its acquisition range; the attack range is the default.
        if def.can_attack()
            && def.base_stat(EntityStatId::ACQUIRE_RANGE).is_none()
            && let Some(range) = def.base_stat(EntityStatId::ATTACK_RANGE)
        {
            def = def.with_stat(EntityStatId::ACQUIRE_RANGE, range);
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
    if let Some(transporter) = optional::<Table>(table, "transporter")? {
        let carries = required::<Vec<String>>(&transporter, "carries")?;
        let boarding = required::<String>(&transporter, "boarding")?;
        let fate = required::<String>(&transporter, "fate")?;
        let conduct = required::<String>(&transporter, "conduct")?;
        def = def.with_transporter(
            carries,
            content::boarding_policy(&boarding)?,
            content::passenger_fate(&fate)?,
            content::passenger_conduct(&conduct)?,
        );
    }
    if let Some(researcher) = optional::<Vec<String>>(table, "researcher")? {
        let ids = researcher
            .iter()
            .map(|name| {
                registry.research(name).ok_or_else(|| {
                    ScriptError::ContentError(format!("research '{name}' is not defined"))
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        def = def.with_researcher(ids);
    }
    if let Some(requires) = optional::<Vec<String>>(table, "requires")? {
        def = def.with_requires(requires);
    }
    if let Some(builder) = optional::<Table>(table, "builder")? {
        let builds = required::<Vec<String>>(&builder, "builds")?;
        let presence = required::<String>(&builder, "presence")?;
        def = def.with_builder(builds, content::work_presence(&presence)?);
    }
    if let Some(repairer) = optional::<Table>(table, "repairer")? {
        let repairs = required::<Vec<String>>(&repairer, "repairs")?;
        let presence = required::<String>(&repairer, "presence")?;
        // Off unless declared, and an omitted patience waits indefinitely.
        let self_repair = optional::<bool>(&repairer, "self_repair")?.unwrap_or(false);
        let patience = optional::<u32>(&repairer, "patience")?;
        def = def.with_repairer(
            repairs,
            parse_repair_rate(&repairer)?,
            content::work_presence(&presence)?,
            self_repair,
            parse_repair_cost(&repairer)?,
            patience,
        );
    }
    if let Some(ratio) = optional::<String>(table, "repair_ratio")? {
        def = def.with_repair_ratio(content::fixed(&ratio)?);
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
    if let Some(projectile) = optional::<String>(table, "projectile")? {
        let id = registry.projectile(&projectile).ok_or_else(|| {
            ScriptError::ContentError(format!("projectile '{projectile}' is not defined"))
        })?;
        def = def.with_projectile(id);
    }
    if let Some(splash) = optional::<Table>(table, "splash")? {
        let shape = required::<String>(&splash, "shape")?;
        let shape = content::splash_shape(&shape)?;
        let bands = splash_bands(&splash)?;
        let layers = required::<u32>(&splash, "layers")?;
        let friendly_fire = required_flag(&splash, "friendly_fire")?;
        def = def.with_splash(shape, bands, LayerMask::from(layers), friendly_fire);
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

/// Reads a repairer's `rate` block: `{ mode = ... }`, where the mode is
/// `"production"` or `"per_tick"` with a `health` amount.
///
/// Required, because the two pace the work so differently that inferring one would
/// hide the choice: a structure mended against its build time and a casualty patched
/// up at a flat rate are both ordinary content.
fn parse_repair_rate(repairer: &Table) -> crate::Result<RepairRate> {
    let rate = required::<Table>(repairer, "rate")?;
    let mode = required::<String>(&rate, "mode")?;
    match mode.as_str() {
        "production" => Ok(RepairRate::Production),
        "per_tick" => {
            let health = required::<String>(&rate, "health")?;
            Ok(RepairRate::PerTick(content::fixed(&health)?))
        }
        other => Err(ScriptError::ContentError(format!(
            "unknown repair rate mode '{other}'"
        ))),
    }
}

/// Reads a repairer's `cost` block: `{ mode = ... }`, where the mode is `"free"`,
/// `"pro_rata"`, `"per_tick"` with a `resources` table of amounts, or `"energy"`
/// with a `per_health` rate.
///
/// Required, and `"free"` has to be said out loud: free work is a balance stance
/// rather than an absence, and inferring it would turn a misspelled field name into
/// unlimited free repair.
fn parse_repair_cost(repairer: &Table) -> crate::Result<RepairCost> {
    let cost = required::<Table>(repairer, "cost")?;
    let mode = required::<String>(&cost, "mode")?;
    match mode.as_str() {
        "free" => Ok(RepairCost::Free),
        "pro_rata" => Ok(RepairCost::ProRata),
        "per_tick" => {
            let resources = required::<Table>(&cost, "resources")?;
            Ok(RepairCost::PerTick(
                pairs::<u32>(&resources, "resources")?.into_iter().collect(),
            ))
        }
        "energy" => {
            let per_health = required::<String>(&cost, "per_health")?;
            Ok(RepairCost::Energy(content::fixed(&per_health)?))
        }
        other => Err(ScriptError::ContentError(format!(
            "unknown repair cost mode '{other}'"
        ))),
    }
}

/// Reads one projectile: `{ speed, aim }` — a decimal string in cells per tick, and
/// whether the hit resolves against the target entity or the cell it was sent to.
fn parse_projectile(table: &Table) -> crate::Result<ProjectileDef> {
    let speed = required::<String>(table, "speed")?;
    let aim = required::<String>(table, "aim")?;
    Ok(ProjectileDef::new(
        content::fixed(&speed)?,
        content::attack_aim(&aim)?,
    ))
}

/// Reads a splash `bands` list: an array of `{ radius, fraction }` pairs, innermost
/// first, where the fraction is a decimal string.
fn splash_bands(splash: &Table) -> crate::Result<Vec<(u32, FixedU64)>> {
    let bands: Vec<Table> = required(splash, "bands")?;
    let mut out = Vec::with_capacity(bands.len());
    for band in bands {
        let radius = band
            .get::<u32>(1)
            .map_err(|error| field_error("splash band radius", error))?;
        let fraction = band
            .get::<String>(2)
            .map_err(|error| field_error("splash band fraction", error))?;
        out.push((radius, content::fixed(&fraction)?));
    }
    Ok(out)
}

/// Reads the flat `stats = { name = value }` table: each key is a registered stat
/// name (built-in, or content-declared with `define_entity_stat`), each value a
/// non-negative integer or a decimal string. An unknown name is rejected — a
/// custom stat must be declared before it is set.
fn parse_stats(
    stats: &Table,
    registry: &ContentRegistry,
) -> crate::Result<Vec<(EntityStatId, FixedU64)>> {
    let mut out = Vec::new();
    for pair in stats.pairs::<String, Value>() {
        let (name, value) = pair.map_err(|error| field_error("stats", error))?;
        let stat = registry
            .entity_stat(&name)
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
fn cell_size(location: &Table) -> crate::Result<CellSize> {
    match required::<Value>(location, "size")? {
        Value::Integer(side) => {
            let side = dimension(side)?;
            Ok(CellSize::new(side, side))
        }
        Value::Table(size) => {
            let width = index(&size, 1)?;
            let height = index(&size, 2)?;
            Ok(CellSize::new(width, height))
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

/// Reads a `resource_carrier` map: each kind maps to `{capacity, time, presence}`.
fn harvest_kinds(carrier: &Table) -> crate::Result<Vec<(String, HarvestData)>> {
    let mut carries = Vec::new();
    for pair in carrier.clone().pairs::<String, Table>() {
        let (kind, data) = pair.map_err(|error| field_error("resource_carrier", error))?;
        let harvest = HarvestData::new(
            required::<u32>(&data, "capacity")?,
            required::<u32>(&data, "time")?,
            content::work_presence(&required::<String>(&data, "presence")?)?,
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

/// A required boolean field.
///
/// Read through `Option` rather than as a plain `bool`, because Lua's `nil`
/// converts to `false` — a direct read cannot tell an absent flag from one that
/// was deliberately set to `false`.
fn required_flag(table: &Table, field: &str) -> crate::Result<bool> {
    optional::<bool>(table, field)?
        .ok_or_else(|| ScriptError::ContentError(format!("field '{field}' is required")))
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

/// Reads one skill: `{ caster, cooldown, ... }` — the `caster` arm decides the
/// remaining fields. An entity cast reads `{ cost?, target, effect }`; a
/// player cast reads `{ cost?, effect }` and takes no target (the cast lands
/// on the casting player). A missing `cost` block is a free skill.
fn parse_skill(table: &Table, registry: &ContentRegistry) -> crate::Result<SkillDef> {
    let cooldown = required::<u32>(table, "cooldown")?;
    let caster = match required::<String>(table, "caster")?.as_str() {
        "entity" => SkillCaster::Entity {
            costs: match optional::<Table>(table, "cost")? {
                Some(cost) => parse_entity_cast_cost(&cost)?,
                None => Vec::new(),
            },
            target: parse_entity_cast_target(&required::<String>(table, "target")?)?,
            effect: parse_entity_effect(&required::<Table>(table, "effect")?, registry)?,
        },
        "player" => {
            if optional::<Value>(table, "target")?.is_some() {
                return Err(ScriptError::ContentError(
                    "a player-cast skill takes no target: the cast lands on the casting player"
                        .to_string(),
                ));
            }
            SkillCaster::Player {
                cost: match optional::<Table>(table, "cost")? {
                    Some(cost) => parse_player_cast_cost(&cost)?,
                    None => Cost::new(),
                },
                effect: parse_player_effect(&required::<Table>(table, "effect")?, registry)?,
            }
        }
        other => {
            return Err(ScriptError::ContentError(format!(
                "unknown skill caster '{other}' (expected entity or player)"
            )));
        }
    };
    let requires = optional::<Vec<String>>(table, "requires")?.unwrap_or_default();
    Ok(SkillDef {
        cooldown,
        caster,
        requires,
    })
}

/// Reads a research definition: a `cost` table of resource amounts, the
/// research `time` in ticks, an optional player `buff` applied on completion,
/// and optional `requires` entries. The buff name resolves in the player-buff
/// registry.
fn parse_research(table: &Table, registry: &ContentRegistry) -> crate::Result<ResearchDef> {
    let cost: Cost = match optional::<Table>(table, "cost")? {
        Some(cost) => pairs::<u32>(&cost, "cost")?.into_iter().collect(),
        None => Cost::new(),
    };
    let time = required::<u32>(table, "time")?;
    let buff = match optional::<String>(table, "buff")? {
        Some(name) => Some(registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?),
        None => None,
    };
    let requires = optional::<Vec<String>>(table, "requires")?.unwrap_or_default();
    Ok(ResearchDef::new(cost, time, buff, requires))
}

/// Reads an entity cast's `cost` block: any of `resources` (a table of
/// amounts), `energy`, and `health` (decimal strings) — each present entry one
/// price a cast pays.
fn parse_entity_cast_cost(cost: &Table) -> crate::Result<Vec<EntityCastCost>> {
    let mut costs = Vec::new();
    if let Some(resources) = optional::<Table>(cost, "resources")? {
        costs.push(EntityCastCost::Resources(
            pairs::<u32>(&resources, "resources")?.into_iter().collect(),
        ));
    }
    if let Some(energy) = optional::<String>(cost, "energy")? {
        costs.push(EntityCastCost::Energy(content::fixed(&energy)?));
    }
    if let Some(health) = optional::<String>(cost, "health")? {
        costs.push(EntityCastCost::Health(content::fixed(&health)?));
    }
    if costs.is_empty() {
        return Err(ScriptError::ContentError(
            "skill cost must name at least one of resources, energy, or health".to_string(),
        ));
    }
    Ok(costs)
}

/// Reads a player cast's `cost` block: a `resources` table of amounts — the
/// only pool a player has.
fn parse_player_cast_cost(cost: &Table) -> crate::Result<Cost> {
    let resources = required::<Table>(cost, "resources")?;
    Ok(pairs::<u32>(&resources, "resources")?.into_iter().collect())
}

fn parse_entity_cast_target(target: &str) -> crate::Result<EntityCastTarget> {
    match target {
        "caster" => Ok(EntityCastTarget::Caster),
        "ally" => Ok(EntityCastTarget::Ally),
        "enemy" => Ok(EntityCastTarget::Enemy),
        other => Err(ScriptError::ContentError(format!(
            "unknown skill target '{other}' (expected caster, ally, or enemy)"
        ))),
    }
}

/// Reads an entity cast's effect: exactly one of `apply_buff`, `remove_buff`,
/// `damage`, `heal`. Buff names resolve in the entity-buff registry.
fn parse_entity_effect(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityCastEffect> {
    if let Some(name) = optional::<String>(table, "apply_buff")? {
        let id = registry.entity_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("entity buff '{name}' is not defined"))
        })?;
        Ok(EntityCastEffect::ApplyBuff(id))
    } else if let Some(name) = optional::<String>(table, "remove_buff")? {
        let id = registry.entity_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("entity buff '{name}' is not defined"))
        })?;
        Ok(EntityCastEffect::RemoveBuff(id))
    } else if let Some(amount) = optional::<String>(table, "damage")? {
        Ok(EntityCastEffect::Damage(content::fixed(&amount)?))
    } else if let Some(amount) = optional::<String>(table, "heal")? {
        Ok(EntityCastEffect::Heal(content::fixed(&amount)?))
    } else {
        Err(ScriptError::ContentError(
            "skill effect must be one of apply_buff, remove_buff, damage, or heal".to_string(),
        ))
    }
}

/// Reads a player cast's effect: exactly one of `apply_buff` and
/// `remove_buff`. Buff names resolve in the player-buff registry.
fn parse_player_effect(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<PlayerCastEffect> {
    if let Some(name) = optional::<String>(table, "apply_buff")? {
        let id = registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?;
        Ok(PlayerCastEffect::ApplyBuff(id))
    } else if let Some(name) = optional::<String>(table, "remove_buff")? {
        let id = registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?;
        Ok(PlayerCastEffect::RemoveBuff(id))
    } else {
        Err(ScriptError::ContentError(
            "player-cast skill effect must be one of apply_buff or remove_buff".to_string(),
        ))
    }
}

/// Reads an entity buff definition: `{ duration?, stack, modifiers }`.
fn parse_entity_buff(table: &Table, registry: &ContentRegistry) -> crate::Result<EntityBuffDef> {
    Ok(EntityBuffDef {
        duration: optional::<u32>(table, "duration")?,
        stack_rule: parse_stack_rule(&required::<String>(table, "stack")?)?,
        modifiers: parse_entity_modifiers(&required::<Vec<Table>>(table, "modifiers")?, registry)?,
    })
}

/// Reads a player buff definition: `{ duration?, stack, player_modifiers?,
/// entity_modifiers? }` — at least one modifier list must be present.
fn parse_player_buff(table: &Table, registry: &ContentRegistry) -> crate::Result<PlayerBuffDef> {
    let player_modifiers = match optional::<Vec<Table>>(table, "player_modifiers")? {
        Some(modifiers) => parse_player_modifiers(&modifiers, registry)?,
        None => Vec::new(),
    };
    let entity_modifiers = match optional::<Vec<Table>>(table, "entity_modifiers")? {
        Some(modifiers) => parse_entity_modifiers(&modifiers, registry)?,
        None => Vec::new(),
    };
    if player_modifiers.is_empty() && entity_modifiers.is_empty() {
        return Err(ScriptError::ContentError(
            "player buff must declare player_modifiers or entity_modifiers".to_string(),
        ));
    }
    Ok(PlayerBuffDef {
        player_modifiers,
        entity_modifiers,
        duration: optional::<u32>(table, "duration")?,
        stack_rule: parse_stack_rule(&required::<String>(table, "stack")?)?,
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

fn parse_entity_modifiers(
    modifiers: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<EntityModifier>> {
    modifiers
        .iter()
        .map(|modifier| parse_entity_modifier(modifier, registry))
        .collect()
}

fn parse_player_modifiers(
    modifiers: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<PlayerModifier>> {
    modifiers
        .iter()
        .map(|modifier| parse_player_modifier(modifier, registry))
        .collect()
}

/// Reads one entity modifier: `{ entity_stat, op, value }`.
fn parse_entity_modifier(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityModifier> {
    if optional::<Value>(table, "player_stat")?.is_some() {
        return Err(ScriptError::ContentError(
            "this modifier list holds entity modifiers; expected entity_stat, found player_stat"
                .to_string(),
        ));
    }
    let name = required::<String>(table, "entity_stat")?;
    let stat = registry
        .entity_stat(&name)
        .ok_or_else(|| ScriptError::ContentError(format!("entity stat '{name}' is not defined")))?;
    let (op, magnitude) = parse_modifier_op_value(table)?;
    Ok(EntityModifier {
        stat,
        op,
        magnitude,
    })
}

/// Reads one player modifier: `{ player_stat, op, value }`.
fn parse_player_modifier(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<PlayerModifier> {
    if optional::<Value>(table, "entity_stat")?.is_some() {
        return Err(ScriptError::ContentError(
            "this modifier list holds player modifiers; expected player_stat, found entity_stat"
                .to_string(),
        ));
    }
    let name = required::<String>(table, "player_stat")?;
    let stat = registry
        .player_stat(&name)
        .ok_or_else(|| ScriptError::ContentError(format!("player stat '{name}' is not defined")))?;
    let (op, magnitude) = parse_modifier_op_value(table)?;
    Ok(PlayerModifier {
        stat,
        op,
        magnitude,
    })
}

/// Reads a modifier's `op` and `value` fields, shared by both modifier kinds.
fn parse_modifier_op_value(table: &Table) -> crate::Result<(ModifierOp, FixedI64)> {
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
    Ok((op, value))
}
